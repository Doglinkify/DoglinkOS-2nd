//! PCI discovery and the bounded xHCI controller reset sequence.

use super::regs;
use super::trb::{
    CompletionCode, TRB_CHAIN, TRB_CYCLE, TRB_TC, TRB_TYPE_COMMAND_COMPLETION, TRB_TYPE_LINK,
    TRB_TYPE_MASK, Trb,
};
use super::usb::{
    HidBootEndpoint, PortState, SetupRequest, SupportedProtocol, get_descriptor,
    parse_configuration, set_configuration, set_idle, supported_protocol, usb2_max_packet,
};
use crate::mm::dma::{DmaBuffer, MmioMapping};
use crate::pcie::enumrate::{Bdf, MemoryBar, PCIConfigSpace, decode_memory_bar, doit};
use core::sync::atomic::{Ordering, fence};

const POLL_LIMIT: usize = 100_000;
// USB transfers wait for device and frame scheduling, unlike the controller
// commands above which QEMU completes immediately. Keep this bounded while
// allowing a full control-transfer scheduling window.
const TRANSFER_POLL_LIMIT: usize = 100_000_000;
const TRANSFER_YIELD_INTERVAL: usize = 1024;
const MIN_BAR_LEN: u64 = 0x1000;
const EVENT_RING_ENTRIES: usize = 32;
const COMMAND_RING_ENTRIES: usize = 4096 / core::mem::size_of::<Trb>();
const TRANSFER_RING_ENTRIES: usize = COMMAND_RING_ENTRIES - 1;

struct RootDevice {
    port: u8,
    state: PortState,
    _input_context: DmaBuffer,
    _output_context: DmaBuffer,
    _ep0_ring: DmaBuffer,
    _device_descriptor: DmaBuffer,
    _configuration_descriptor: DmaBuffer,
    _interrupt_ring: DmaBuffer,
    _hid_endpoint: HidBootEndpoint,
}

/// Emit the stable endpoint diagnostic required by the enumeration logs.
/// The configuration stage calls this after validating the complete
/// descriptor and before issuing Configure Endpoint.
#[allow(dead_code)]
fn log_hid_endpoint(slot: u8, endpoint: HidBootEndpoint) {
    crate::println!(
        "[INFO] xhci: HID Boot endpoint slot {}, interface {}, address {:#04x}, max packet {}, interval {}",
        slot,
        endpoint.interface_number,
        endpoint.endpoint_address,
        endpoint.max_packet_size,
        endpoint.interval
    );
}

fn setup_value(request: SetupRequest) -> u64 {
    request.bm_request_type as u64
        | (request.request as u64) << 8
        | (request.value as u64) << 16
        | (request.index as u64) << 32
        | (request.length as u64) << 48
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerState {
    Discovered,
    Reset,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResetError {
    Timeout(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InitError {
    Allocation,
    Timeout(&'static str),
    Command(CompletionCode),
}

#[repr(C)]
#[allow(dead_code)] // Written as the xHCI-defined 16-byte DMA layout.
struct ErstEntry {
    base: u64,
    size: u16,
    _reserved: u16,
    _reserved2: u32,
}

const _: () = {
    assert!(core::mem::size_of::<ErstEntry>() == 16);
    assert!(core::mem::align_of::<ErstEntry>() == core::mem::align_of::<u64>());
};

/// Resources remain owned by this object for as long as the controller runs.
/// There is no IOMMU in this release, so every address handed to xHCI is a
/// physical address from the contiguous DMA allocator.
struct ControllerResources {
    _mapping: MmioMapping,
    dcbaa: DmaBuffer,
    _scratchpad_array: Option<DmaBuffer>,
    _scratchpads: alloc::vec::Vec<DmaBuffer>,
    command_ring: DmaBuffer,
    event_ring: DmaBuffer,
    erst: DmaBuffer,
    event_consumer: usize,
    event_cycle: bool,
    event_count: usize,
    command_producer: usize,
    command_cycle: bool,
    runtime: usize,
    doorbell: usize,
    op: usize,
    context_64: bool,
    ports: alloc::vec::Vec<RootDevice>,
}

impl ControllerResources {
    fn new(mapping: MmioMapping, hcs2: u32) -> Result<Self, InitError> {
        let scratch_count = (((hcs2 >> 27) & 0x1f) << 5 | ((hcs2 >> 21) & 0x1f)) as usize;
        let dcbaa = DmaBuffer::new(256 * 8, 64).map_err(|_| InitError::Allocation)?;
        let scratchpad_array = if scratch_count == 0 {
            None
        } else {
            Some(DmaBuffer::new(scratch_count * 8, 64).map_err(|_| InitError::Allocation)?)
        };
        let mut scratchpads = alloc::vec::Vec::new();
        for index in 0..scratch_count {
            let page = DmaBuffer::new(4096, 4096).map_err(|_| InitError::Allocation)?;
            unsafe {
                core::ptr::write_volatile(
                    (scratchpad_array.as_ref().unwrap().as_ptr() as *mut u64).add(index),
                    page.physical_address(),
                );
            }
            scratchpads.push(page);
        }
        let command_ring = DmaBuffer::new(4096, 64).map_err(|_| InitError::Allocation)?;
        let event_ring =
            DmaBuffer::new(EVENT_RING_ENTRIES * 16, 64).map_err(|_| InitError::Allocation)?;
        let erst = DmaBuffer::new(core::mem::size_of::<ErstEntry>(), 64)
            .map_err(|_| InitError::Allocation)?;
        unsafe {
            let command = command_ring.as_ptr() as *mut Trb;
            core::ptr::write_volatile(
                command.add(COMMAND_RING_ENTRIES - 1),
                Trb {
                    parameter: command_ring.physical_address(),
                    control: TRB_TYPE_LINK | TRB_TC | TRB_CYCLE,
                    ..Trb::default()
                },
            );
            let entry = erst.as_ptr() as *mut ErstEntry;
            core::ptr::write_volatile(
                entry,
                ErstEntry {
                    base: event_ring.physical_address(),
                    size: EVENT_RING_ENTRIES as u16,
                    _reserved: 0,
                    _reserved2: 0,
                },
            );
            if let Some(array) = scratchpad_array.as_ref() {
                core::ptr::write_volatile(dcbaa.as_ptr() as *mut u64, array.physical_address());
            }
        }
        Ok(Self {
            _mapping: mapping,
            dcbaa,
            _scratchpad_array: scratchpad_array,
            _scratchpads: scratchpads,
            command_ring,
            event_ring,
            erst,
            event_consumer: 0,
            event_cycle: true,
            event_count: 0,
            command_producer: 0,
            command_cycle: true,
            runtime: 0,
            doorbell: 0,
            op: 0,
            context_64: false,
            ports: alloc::vec::Vec::new(),
        })
    }

    fn start(
        mut self,
        base: *mut u8,
        cap_len: usize,
        max_slots: u8,
        context_64: bool,
    ) -> Result<Self, InitError> {
        let register = unsafe { regs::RegisterBlock::new(base) };
        let runtime = unsafe { register.read32(regs::RTSOFF) } as usize & !0x1f;
        let doorbell = unsafe { register.read32(regs::DBOFF) } as usize & !3;
        let op = cap_len;
        unsafe {
            register.write64(op + regs::DCBAAP, self.dcbaa.physical_address());
            register.write64(op + regs::CRCR, self.command_ring.physical_address() | 1);
            register.write32(op + regs::CONFIG, max_slots as u32);
            register.write32(runtime + regs::RT_INTR0 + regs::ERSTSZ, 1);
            register.write64(
                runtime + regs::RT_INTR0 + regs::ERSTBA,
                self.erst.physical_address(),
            );
            register.write64(
                runtime + regs::RT_INTR0 + regs::ERDP,
                self.event_ring.physical_address(),
            );
            // Keep event production enabled; events are still consumed by the
            // bounded polling path rather than an interrupt handler.
            register.write32(
                runtime + regs::RT_INTR0 + regs::IMAN,
                regs::IMAN_IP | regs::IMAN_IE,
            );
            register.write32(
                op + regs::USBCMD,
                register.read32(op + regs::USBCMD) | regs::USBCMD_RUN,
            );
        }
        if !wait_for(
            || unsafe { register.read32(op + regs::USBSTS) & regs::USBSTS_HCHALTED == 0 },
            POLL_LIMIT,
        ) {
            return Err(InitError::Timeout("run"));
        }
        self.runtime = runtime;
        self.doorbell = doorbell;
        self.op = op;
        self.context_64 = context_64;
        let completion = self.submit_command(
            register,
            Trb {
                control: regs::TRB_TYPE_NOOP_COMMAND,
                ..Trb::default()
            },
        );
        match completion {
            Ok(_) => {
                crate::println!(
                    "[INFO] xhci: running, MaxSlots {}, context {}, scratchpads {}, command completion",
                    max_slots,
                    if context_64 { 64 } else { 32 },
                    self._scratchpads.len()
                );
                self.enumerate_root_ports(register);
                Ok(self)
            }
            Err(error) => Err(error),
        }
    }

    fn poll_command(
        &mut self,
        register: regs::RegisterBlock,
        command_address: u64,
    ) -> Result<u8, InitError> {
        for _ in 0..POLL_LIMIT {
            let trb = unsafe {
                core::ptr::read_volatile(
                    self.event_ring.as_ptr().add(self.event_consumer * 16) as *const Trb
                )
            };
            if (trb.control & TRB_CYCLE != 0) != self.event_cycle {
                core::hint::spin_loop();
                continue;
            }
            self.event_consumer += 1;
            if self.event_consumer == EVENT_RING_ENTRIES {
                self.event_consumer = 0;
                self.event_cycle = !self.event_cycle;
            }
            self.event_count += 1;
            unsafe {
                register.write64(
                    self.runtime + regs::RT_INTR0 + regs::ERDP,
                    self.event_ring.physical_address() + (self.event_consumer * 16) as u64
                        | regs::ERDP_EHB,
                );
                register.write32(self.runtime + regs::RT_INTR0 + regs::IMAN, regs::IMAN_IP);
                register.write32(self.op + regs::USBSTS, regs::USBSTS_EINT);
            }
            if let Some(code) = command_completion(trb, command_address) {
                return if code == CompletionCode::Success {
                    Ok((trb.control >> 24) as u8)
                } else {
                    crate::println!(
                        "[WARN] xhci: command completion status {:#010x}, code {:#x}",
                        trb.status,
                        trb.status >> 24
                    );
                    Err(InitError::Command(code))
                };
            }
        }
        Err(InitError::Timeout("command completion"))
    }

    fn poll_transfer(
        &mut self,
        register: regs::RegisterBlock,
        trb_address: u64,
    ) -> Result<(), InitError> {
        for poll in 0..TRANSFER_POLL_LIMIT {
            // Enumeration runs before the scheduler enables interrupts.  Poll
            // the controller itself periodically so a virtual xHCI device can
            // observe the doorbell and advance its USB-frame work; a write to
            // an unrelated legacy port does not provide that guarantee.
            if poll % TRANSFER_YIELD_INTERVAL == 0 {
                unsafe { register.read32(self.op + regs::USBSTS) };
            }
            let event = unsafe {
                core::ptr::read_volatile(
                    self.event_ring.as_ptr().add(self.event_consumer * 16) as *const Trb
                )
            };
            if (event.control & TRB_CYCLE != 0) != self.event_cycle {
                core::hint::spin_loop();
                continue;
            }
            self.event_consumer = (self.event_consumer + 1) % EVENT_RING_ENTRIES;
            if self.event_consumer == 0 {
                self.event_cycle = !self.event_cycle;
            }
            unsafe {
                register.write64(
                    self.runtime + regs::RT_INTR0 + regs::ERDP,
                    self.event_ring.physical_address() + (self.event_consumer * 16) as u64
                        | regs::ERDP_EHB,
                );
                register.write32(self.runtime + regs::RT_INTR0 + regs::IMAN, regs::IMAN_IP);
                register.write32(self.op + regs::USBSTS, regs::USBSTS_EINT);
            }
            // The normal parameter is the TRB carrying IOC.  Accept any TRB
            // in this control TD as well: older QEMU/xHCI combinations have
            // been observed to report the data-stage address instead of the
            // final status-stage address.  There is only one outstanding TD
            // on EP0, so this remains unambiguous while avoiding a lost event.
            let transfer_event =
                event.control & TRB_TYPE_MASK == super::trb::TRB_TYPE_TRANSFER_EVENT;
            let expected = event.parameter == trb_address
                || (event.parameter >= trb_address.saturating_sub(2 * 16)
                    && event.parameter <= trb_address);
            if transfer_event && expected {
                return match CompletionCode::from_status(event.status) {
                    CompletionCode::Success => Ok(()),
                    code => Err(InitError::Command(code)),
                };
            }
        }
        crate::println!(
            "[WARN] xhci: transfer timeout, TRB {:#x}, USBSTS {:#010x}",
            trb_address,
            unsafe { register.read32(self.op + regs::USBSTS) },
        );
        Err(InitError::Timeout("device descriptor"))
    }

    fn submit_command(
        &mut self,
        register: regs::RegisterBlock,
        mut command: Trb,
    ) -> Result<u8, InitError> {
        let index = self.command_producer;
        command.control = (command.control & !TRB_CYCLE) | self.command_cycle as u32;
        unsafe {
            core::ptr::write_volatile((self.command_ring.as_ptr() as *mut Trb).add(index), command);
            register.write32(self.doorbell, regs::DB_TARGET);
        }
        self.command_producer += 1;
        if self.command_producer == COMMAND_RING_ENTRIES - 1 {
            self.command_producer = 0;
            self.command_cycle = !self.command_cycle;
            unsafe {
                core::ptr::write_volatile(
                    (self.command_ring.as_ptr() as *mut Trb).add(COMMAND_RING_ENTRIES - 1),
                    Trb {
                        parameter: self.command_ring.physical_address(),
                        control: TRB_TYPE_LINK | TRB_TC | self.command_cycle as u32,
                        ..Trb::default()
                    },
                );
            }
        }
        self.poll_command(
            register,
            self.command_ring.physical_address() + (index * 16) as u64,
        )
    }

    fn supported_protocols(
        &self,
        register: regs::RegisterBlock,
    ) -> alloc::vec::Vec<SupportedProtocol> {
        let hcc1 = unsafe { register.read32(regs::HCCPARAMS1) };
        let mut offset =
            (((hcc1 & regs::HCCPARAMS1_XECP_MASK) >> regs::HCCPARAMS1_XECP_SHIFT) as usize) * 4;
        let mut protocols = alloc::vec::Vec::new();
        for _ in 0..256 {
            if offset == 0 {
                break;
            }
            let header = unsafe { register.read32(offset) };
            if header & regs::XECAP_ID_MASK == 2 {
                if let Some(protocol) =
                    supported_protocol(header, unsafe { register.read32(offset + 8) })
                {
                    crate::println!(
                        "[INFO] xhci: protocol USB {}.{} maps root ports {}..{}",
                        protocol.major,
                        protocol.minor,
                        protocol.port_start,
                        protocol.port_start.saturating_add(protocol.port_count - 1),
                    );
                    protocols.push(protocol);
                }
            }
            let next = ((header & regs::XECAP_NEXT_MASK) >> regs::XECAP_NEXT_SHIFT) as usize * 4;
            if next == 0 || next == offset {
                break;
            }
            offset = offset.saturating_add(next);
        }
        protocols
    }

    fn enumerate_root_ports(&mut self, register: regs::RegisterBlock) {
        let mut protocols = self.supported_protocols(register);
        // Some firmware/QEMU combinations leave the Supported Protocol
        // capability list empty even though the controller exposes root
        // ports.  Keep enumeration useful in that case by treating the
        // controller's ports as USB 2 candidates; the speed field below
        // filters out SuperSpeed ports before reset/addressing.
        if protocols.is_empty() {
            let hcs1 = unsafe { register.read32(regs::HCSPARAMS1) };
            let port_count = (hcs1 >> 24) as u8;
            crate::println!(
                "[WARN] xhci: no Supported Protocol capability, probing {} root ports",
                port_count
            );
            if port_count != 0 {
                protocols.push(SupportedProtocol {
                    major: 2,
                    minor: 0,
                    port_start: 1,
                    port_count,
                    usb2: true,
                });
            }
        }
        for protocol in protocols {
            if !protocol.usb2 {
                crate::println!(
                    "[INFO] xhci: skipping USB {}.{} root ports {}..{}",
                    protocol.major,
                    protocol.minor,
                    protocol.port_start,
                    protocol.port_start.saturating_add(protocol.port_count - 1)
                );
                continue;
            }
            for port in protocol.port_start..protocol.port_start.saturating_add(protocol.port_count)
            {
                if self.ports.iter().any(|device| device.port == port) {
                    continue;
                }
                let offset = self.op + regs::PORTSC + (port as usize - 1) * 0x10;
                let portsc = unsafe { register.read32(offset) };
                if portsc & regs::PORTSC_CCS == 0 {
                    continue;
                }
                let speed = ((portsc >> 10) & 0xf) as u8;
                if speed >= 4 {
                    crate::println!(
                        "[INFO] xhci: port {} connected at SuperSpeed {}, skipping",
                        port,
                        speed
                    );
                    continue;
                }
                crate::println!("[INFO] xhci: port {} connected, resetting", port);
                if !self.reset_port(register, offset) {
                    crate::println!("[WARN] xhci: port {} reset failed", port);
                    continue;
                }
                let portsc = unsafe { register.read32(offset) };
                let speed = ((portsc >> 10) & 0xf) as u8;
                crate::println!("[INFO] xhci: port {} reset complete, speed {}", port, speed);
                match self.address_port(register, port, speed) {
                    Ok(device) => {
                        let desc = unsafe {
                            core::slice::from_raw_parts(device._device_descriptor.as_ptr(), 18)
                        };
                        let vendor = u16::from_le_bytes([desc[8], desc[9]]);
                        let product = u16::from_le_bytes([desc[10], desc[11]]);
                        crate::println!(
                            "[INFO] xhci: port {}, slot {}, speed {}, EP0 max packet {}, {:04x}:{:04x}",
                            device.port,
                            match device.state {
                                PortState::Addressed { slot, .. } => slot,
                                _ => 0,
                            },
                            speed,
                            desc[7],
                            vendor,
                            product,
                        );
                        self.ports.push(device);
                    }
                    Err(error) => crate::println!(
                        "[WARN] xhci: port {} enumeration failed: {:?}",
                        port,
                        error
                    ),
                }
            }
        }
    }

    fn reset_port(&self, register: regs::RegisterBlock, offset: usize) -> bool {
        let value = unsafe { register.read32(offset) };
        unsafe {
            register.write32(
                offset,
                (value & regs::PORTSC_PP) | regs::PORTSC_PR | regs::PORTSC_CSC | regs::PORTSC_PRC,
            )
        };
        wait_for(
            || unsafe {
                register.read32(offset) & regs::PORTSC_PR == 0
                    && register.read32(offset) & regs::PORTSC_PED != 0
            },
            POLL_LIMIT,
        )
    }

    fn address_port(
        &mut self,
        register: regs::RegisterBlock,
        port: u8,
        speed: u8,
    ) -> Result<RootDevice, InitError> {
        let slot = match self.submit_command(
            register,
            Trb {
                control: regs::TRB_TYPE_ENABLE_SLOT,
                ..Trb::default()
            },
        ) {
            Ok(slot) => slot,
            Err(error) => {
                crate::println!("[WARN] xhci: port {} Enable Slot failed: {:?}", port, error);
                return Err(error);
            }
        };
        if slot == 0 {
            crate::println!("[WARN] xhci: port {} Enable Slot returned slot 0", port);
            return Err(InitError::Command(CompletionCode::Invalid));
        }
        crate::println!(
            "[INFO] xhci: port {} Enable Slot completed, slot {}",
            port,
            slot
        );
        let context_size = if self.context_64 { 64 } else { 32 };
        let input = DmaBuffer::new(context_size * 33, 64).map_err(|_| InitError::Allocation)?;
        let output = DmaBuffer::new(context_size * 32, 64).map_err(|_| InitError::Allocation)?;
        let ep0 = DmaBuffer::new(4096, 64).map_err(|_| InitError::Allocation)?;
        let descriptor = DmaBuffer::new(18, 64).map_err(|_| InitError::Allocation)?;
        let mut ep0_producer = 0;
        unsafe {
            core::ptr::write_volatile(
                (self.dcbaa.as_ptr() as *mut u64).add(slot as usize),
                output.physical_address(),
            );
            core::ptr::write_volatile(input.as_ptr().add(4) as *mut u32, 0b11);
            core::ptr::write_volatile(
                input.as_ptr().add(context_size) as *mut u32,
                (speed as u32) << 20 | 1 << 27,
            );
            core::ptr::write_volatile(
                input.as_ptr().add(context_size + 4) as *mut u32,
                (port as u32) << 16,
            );
            core::ptr::write_volatile(
                input.as_ptr().add(context_size * 2 + 4) as *mut u32,
                // EP0 is a bidirectional control endpoint.  CErr must be 3
                // for control transfers; a zero value is rejected by xHCI.
                (usb2_max_packet(speed) as u32) << 16 | 4 << 3 | 3 << 1,
            );
            core::ptr::write_volatile(
                input.as_ptr().add(context_size * 2 + 8) as *mut u64,
                ep0.physical_address() | 1,
            );
            core::ptr::write_volatile(
                (ep0.as_ptr() as *mut Trb).add(COMMAND_RING_ENTRIES - 1),
                Trb {
                    parameter: ep0.physical_address(),
                    control: TRB_TYPE_LINK | TRB_TC | TRB_CYCLE,
                    ..Trb::default()
                },
            );
        }
        // Publish DCBAA, input contexts, and the initial endpoint ring before
        // the Address Device command lets the controller fetch them.
        fence(Ordering::SeqCst);
        if let Err(error) = self.submit_command(
            register,
            Trb {
                parameter: input.physical_address(),
                control: regs::TRB_TYPE_ADDRESS_DEVICE | (slot as u32) << 24,
                ..Trb::default()
            },
        ) {
            crate::println!(
                "[WARN] xhci: port {}, slot {} Address Device failed: {:?}",
                port,
                slot,
                error
            );
            return Err(error);
        }
        crate::println!("[INFO] xhci: port {}, slot {} addressed", port, slot);
        self.log_ep0_context(slot, &output);
        crate::println!(
            "[INFO] xhci: port {}, slot {} HID configuration scan pending (SET_CONFIGURATION/Configure Endpoint)",
            port,
            slot
        );
        if let Err(error) =
            self.read_device_descriptor(register, slot, &ep0, &mut ep0_producer, &descriptor)
        {
            self.log_ep0_context(slot, &output);
            crate::println!(
                "[WARN] xhci: port {}, slot {} Device Descriptor failed: {:?}",
                port,
                slot,
                error
            );
            return Err(error);
        }
        let config_header = DmaBuffer::new(9, 64).map_err(|_| InitError::Allocation)?;
        self.control_transfer(
            register,
            slot,
            &ep0,
            &mut ep0_producer,
            get_descriptor(2, 9),
            Some(&config_header),
        )?;
        let header = unsafe { core::slice::from_raw_parts(config_header.as_ptr(), 9) };
        let total = u16::from_le_bytes([header[2], header[3]]) as usize;
        if total < 9 || total > 4096 {
            return Err(InitError::Command(CompletionCode::TrbError));
        }
        let configuration = DmaBuffer::new(total, 64).map_err(|_| InitError::Allocation)?;
        self.control_transfer(
            register,
            slot,
            &ep0,
            &mut ep0_producer,
            get_descriptor(2, total as u16),
            Some(&configuration),
        )?;
        let endpoint = parse_configuration(unsafe {
            core::slice::from_raw_parts(configuration.as_ptr(), total)
        })
        .map_err(|_| InitError::Command(CompletionCode::TrbError))?;
        log_hid_endpoint(slot, endpoint);
        self.control_transfer(
            register,
            slot,
            &ep0,
            &mut ep0_producer,
            set_configuration(endpoint.configuration_value),
            None,
        )?;
        self.control_transfer(
            register,
            slot,
            &ep0,
            &mut ep0_producer,
            set_idle(endpoint.interface_number),
            None,
        )?;
        let interrupt_ring = DmaBuffer::new(4096, 64).map_err(|_| InitError::Allocation)?;
        self.configure_endpoint(register, slot, &input, endpoint, &interrupt_ring)?;
        crate::println!(
            "[INFO] xhci: HID endpoint configured slot {}, endpoint {:#04x}, interval {}, max packet {}, DCS 1",
            slot,
            endpoint.endpoint_address,
            endpoint.interval,
            endpoint.max_packet_size
        );
        Ok(RootDevice {
            port,
            state: PortState::Addressed {
                slot,
                speed,
                max_packet: usb2_max_packet(speed),
            },
            _input_context: input,
            _output_context: output,
            _ep0_ring: ep0,
            _device_descriptor: descriptor,
            _configuration_descriptor: configuration,
            _interrupt_ring: interrupt_ring,
            _hid_endpoint: endpoint,
        })
    }

    fn log_ep0_context(&self, slot: u8, output: &DmaBuffer) {
        let context_size = if self.context_64 { 64 } else { 32 };
        unsafe {
            // A device context starts directly with the slot context.  Unlike
            // an input context, it has no leading input-control context, so
            // EP0 is context 1 rather than context 2.
            let context = output.as_ptr().add(context_size) as *const u32;
            let state = core::ptr::read_volatile(context) & 0x7;
            let max_packet = core::ptr::read_volatile(context.add(1)) >> 16;
            let dequeue = core::ptr::read_volatile(context.add(2) as *const u64);
            crate::println!(
                "[DEBUG] xhci: slot {} EP0 state {}, max packet {}, dequeue {:#x}",
                slot,
                state,
                max_packet,
                dequeue,
            );
        }
    }

    fn control_transfer(
        &mut self,
        register: regs::RegisterBlock,
        slot: u8,
        ring: &DmaBuffer,
        producer: &mut usize,
        request: SetupRequest,
        data: Option<&DmaBuffer>,
    ) -> Result<(), InitError> {
        let trb_count = if data.is_some() { 3 } else { 2 };
        if *producer + trb_count > TRANSFER_RING_ENTRIES {
            // Enumeration submits only a few TDs, but do not overwrite a TD
            // that the controller may still own if that ever changes.
            return Err(InitError::Command(CompletionCode::RingOverrun));
        }
        let ptr = unsafe { (ring.as_ptr() as *mut Trb).add(*producer) };
        let data_len = request.length as u32;
        let data_in = request.bm_request_type & 0x80 != 0;
        let transfer_type = if data.is_some() {
            if data_in { 3 } else { 2 }
        } else {
            0
        };
        unsafe {
            core::ptr::write_volatile(
                ptr,
                Trb {
                    parameter: setup_value(request),
                    status: 8,
                    control: super::trb::TRB_TYPE_SETUP_STAGE
                        | (transfer_type << 16)
                        // The setup packet is in this TRB's parameter field.
                        | (1 << 6)
                        | TRB_CHAIN
                        | TRB_CYCLE,
                },
            );
            if let Some(buffer) = data {
                core::ptr::write_volatile(
                    ptr.add(1),
                    Trb {
                        parameter: buffer.physical_address(),
                        status: data_len,
                        control: super::trb::TRB_TYPE_DATA_STAGE
                            | if data_in { 1 << 16 } else { 0 }
                            | TRB_CHAIN
                            | TRB_CYCLE,
                    },
                );
                core::ptr::write_volatile(
                    ptr.add(2),
                    Trb {
                        // IOC on the final TRB is what produces the transfer
                        // completion event consumed by poll_transfer.
                        control: super::trb::TRB_TYPE_STATUS_STAGE
                            | if data_in { 0 } else { 1 << 16 }
                            | (1 << 5)
                            | TRB_CYCLE,
                        ..Trb::default()
                    },
                );
                fence(Ordering::SeqCst);
                register.write32(self.doorbell + slot as usize * 4, 1);
                let completion = self.poll_transfer(
                    register,
                    ring.physical_address() + ((*producer + 2) * 16) as u64,
                );
                *producer += trb_count;
                return completion;
            }
            core::ptr::write_volatile(
                ptr.add(1),
                Trb {
                    // A no-data control request has an IN status stage.
                    control: super::trb::TRB_TYPE_STATUS_STAGE | (1 << 16) | (1 << 5) | TRB_CYCLE,
                    ..Trb::default()
                },
            );
            fence(Ordering::SeqCst);
            register.write32(self.doorbell + slot as usize * 4, 1);
        }
        let completion = self.poll_transfer(
            register,
            ring.physical_address() + ((*producer + 1) * 16) as u64,
        );
        *producer += trb_count;
        completion
    }

    fn configure_endpoint(
        &mut self,
        register: regs::RegisterBlock,
        slot: u8,
        input: &DmaBuffer,
        endpoint: HidBootEndpoint,
        ring: &DmaBuffer,
    ) -> Result<(), InitError> {
        let context_size = if self.context_64 { 64 } else { 32 };
        let endpoint_id = ((endpoint.endpoint_address & 0x0f) as usize) * 2
            + usize::from(endpoint.endpoint_address & 0x80 != 0);
        if endpoint_id == 0 || endpoint_id >= 32 {
            return Err(InitError::Command(CompletionCode::TrbError));
        }
        unsafe {
            let control = input.as_ptr().add(4) as *mut u32;
            // Address Device used the same buffer with EP0 in its Add Context
            // Flags.  Configure Endpoint accepts only the contexts this
            // command changes: the Slot Context and the new interrupt EP.
            core::ptr::write_volatile(control, 1 | 1 << endpoint_id);

            // The Input Context has an Input Control Context before the slot
            // and endpoint contexts.  The slot must also advertise the
            // highest endpoint-context index being configured.
            let slot_context = input.as_ptr().add(context_size) as *mut u32;
            let slot_context_value = core::ptr::read_volatile(slot_context);
            core::ptr::write_volatile(
                slot_context,
                (slot_context_value & !(0x1f << 27)) | (endpoint_id as u32) << 27,
            );
            let context = input.as_ptr().add(context_size * (endpoint_id + 1)) as *mut u32;
            core::ptr::write_volatile(context, (endpoint.interval.saturating_sub(1) as u32) << 16);
            core::ptr::write_volatile(
                context.add(1),
                (endpoint.max_packet_size as u32) << 16 | 3 << 1 | 7 << 3,
            );
            core::ptr::write_volatile(context.add(2) as *mut u64, ring.physical_address() | 1);
            core::ptr::write_volatile(
                (ring.as_ptr() as *mut Trb).add(COMMAND_RING_ENTRIES - 1),
                Trb {
                    parameter: ring.physical_address(),
                    control: TRB_TYPE_LINK | TRB_TC | TRB_CYCLE,
                    ..Trb::default()
                },
            );
            crate::println!(
                "[DEBUG] xhci: Configure Endpoint slot {}, add {:#010x}, entries {}, endpoint context {:#x}",
                slot,
                core::ptr::read_volatile(control),
                core::ptr::read_volatile(slot_context) >> 27,
                ring.physical_address() | 1,
            );
        }
        fence(Ordering::SeqCst);
        self.submit_command(
            register,
            Trb {
                parameter: input.physical_address(),
                control: regs::TRB_TYPE_CONFIGURE_ENDPOINT | (slot as u32) << 24,
                ..Trb::default()
            },
        )
        .map(|_| ())
    }

    fn read_device_descriptor(
        &mut self,
        register: regs::RegisterBlock,
        slot: u8,
        ring: &DmaBuffer,
        producer: &mut usize,
        descriptor: &DmaBuffer,
    ) -> Result<(), InitError> {
        self.control_transfer(
            register,
            slot,
            ring,
            producer,
            get_descriptor(1, 18),
            Some(descriptor),
        )
    }
}

fn command_completion(event: Trb, command_address: u64) -> Option<CompletionCode> {
    (event.control & TRB_TYPE_MASK == TRB_TYPE_COMMAND_COMPLETION
        && event.parameter == command_address)
        .then(|| CompletionCode::from_status(event.status))
}

fn wait_for<F: FnMut() -> bool>(mut ready: F, limit: usize) -> bool {
    for _ in 0..limit {
        if ready() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn bar0(config: &PCIConfigSpace) -> Option<(u64, u64)> {
    let low = config.read_u32(0x10);
    let high = config.read_u32(0x14);
    let decoded = decode_memory_bar(low, high)?;
    let address = match decoded {
        MemoryBar::Bits32 { address } | MemoryBar::Bits64 { address } => address,
    };
    // Probe the BAR size using the standard PCI sizing transaction. Restore it
    // before touching the controller; a zero mask means the BAR is unusable.
    unsafe {
        config.write_u32(0x10, 0xffff_ffff);
    }
    let mask_low = config.read_u32(0x10);
    let mask_high = if matches!(decoded, MemoryBar::Bits64 { .. }) {
        unsafe {
            config.write_u32(0x14, 0xffff_ffff);
        }
        config.read_u32(0x14)
    } else {
        0
    };
    unsafe {
        config.write_u32(0x10, low);
        if matches!(decoded, MemoryBar::Bits64 { .. }) {
            config.write_u32(0x14, high);
        }
    }
    let mask = (mask_low as u64 & 0xffff_fff0) | ((mask_high as u64) << 32);
    let length = (!mask).wrapping_add(1);
    if address == 0 || length < MIN_BAR_LEN || !length.is_power_of_two() {
        return None;
    }
    Some((address, length))
}

fn ownership(regs: regs::RegisterBlock, xecp: usize) -> bool {
    let mut offset = xecp;
    for _ in 0..256 {
        let header = unsafe { regs.read32(offset) };
        let id = header & regs::XECAP_ID_MASK;
        let next = ((header & regs::XECAP_NEXT_MASK) >> regs::XECAP_NEXT_SHIFT) as usize * 4;
        if id == regs::XECAP_USB_LEGACY_SUPPORT {
            let value = unsafe { regs.read32(offset) };
            if value & regs::USBLEGSUP_BIOS_OWNED != 0 {
                unsafe {
                    regs.write32(offset, value | regs::USBLEGSUP_OS_OWNED);
                }
                return wait_for(
                    || unsafe { regs.read32(offset) & regs::USBLEGSUP_BIOS_OWNED == 0 },
                    POLL_LIMIT,
                );
            }
            return true;
        }
        if next == 0 || next == offset {
            break;
        }
        offset = offset.saturating_add(next);
    }
    true
}

fn reset_controller(base: *mut u8) -> Result<(u16, u8, u32, bool, usize), ResetError> {
    let regs = unsafe { regs::RegisterBlock::new(base) };
    let cap_len = unsafe { regs.read32(regs::CAPLENGTH) } as usize & 0xff;
    let version = unsafe { regs.read16(regs::HCIVERSION) };
    let hcs1 = unsafe { regs.read32(regs::HCSPARAMS1) };
    let hcs2 = unsafe { regs.read32(regs::HCSPARAMS1 + 4) };
    let hcc1 = unsafe { regs.read32(regs::HCCPARAMS1) };
    let port_count = (hcs1 >> 24) as u8;
    // xECP is expressed in DWORDs from the capability base.
    let xecp = (((hcc1 & regs::HCCPARAMS1_XECP_MASK) >> regs::HCCPARAMS1_XECP_SHIFT) as usize)
        .saturating_mul(4);
    if cap_len < 0x20 || !ownership(regs, xecp) {
        return Err(ResetError::Timeout("BIOS ownership"));
    }
    let op = cap_len;
    unsafe {
        regs.write32(
            op + regs::USBCMD,
            regs.read32(op + regs::USBCMD) & !regs::USBCMD_RUN,
        );
    }
    if !wait_for(
        || unsafe { regs.read32(op + regs::USBSTS) & regs::USBSTS_HCHALTED != 0 },
        POLL_LIMIT,
    ) {
        return Err(ResetError::Timeout("halt"));
    }
    unsafe {
        regs.write32(
            op + regs::USBCMD,
            regs.read32(op + regs::USBCMD) | regs::USBCMD_HCRST,
        );
    }
    if !wait_for(
        || unsafe { regs.read32(op + regs::USBCMD) & regs::USBCMD_HCRST == 0 },
        POLL_LIMIT,
    ) {
        return Err(ResetError::Timeout("reset"));
    }
    if !wait_for(
        || unsafe { regs.read32(op + regs::USBSTS) & regs::USBSTS_CNR == 0 },
        POLL_LIMIT,
    ) {
        return Err(ResetError::Timeout("CNR"));
    }
    Ok((version, port_count, hcs2, hcc1 & (1 << 2) != 0, cap_len))
}

fn discover_one(bdf: Bdf, config: &PCIConfigSpace) -> ControllerState {
    if config.class_code != 0x0c || config.subclass != 0x03 || config.prog_if != 0x30 {
        return ControllerState::Failed;
    }
    let Some((address, length)) = bar0(config) else {
        crate::println!(
            "[WARN] xhci: {:02x}:{:02x}.{} reset failed at BAR0",
            bdf.bus,
            bdf.device,
            bdf.function
        );
        return ControllerState::Failed;
    };
    unsafe {
        config.update_command(1 << 1 | 1 << 2, 0);
    }
    let mapping = match unsafe { MmioMapping::map(address, length as usize) } {
        Ok(mapping) => mapping,
        Err(error) => {
            crate::println!(
                "[WARN] xhci: {:02x}:{:02x}.{} reset failed at MMIO mapping (BAR0 {:#x}, len {:#x}, {:?})",
                bdf.bus,
                bdf.device,
                bdf.function,
                address,
                length,
                error,
            );
            return ControllerState::Failed;
        }
    };
    match reset_controller(mapping.as_ptr()) {
        Ok((version, ports, hcs2, context_64, cap_len)) => {
            crate::println!(
                "[INFO] xhci: {:02x}:{:02x}.{} version {:x}, ports {}, reset successful",
                bdf.bus,
                bdf.device,
                bdf.function,
                version,
                ports
            );
            let mapping_ptr = mapping.as_ptr();
            let max_slots =
                (unsafe { regs::RegisterBlock::new(mapping_ptr).read32(regs::HCSPARAMS1) } & 0xff)
                    as u8;
            match ControllerResources::new(mapping, hcs2)
                .and_then(|resources| resources.start(mapping_ptr, cap_len, max_slots, context_64))
            {
                Ok(resources) => {
                    // The controller owns these DMA buffers until shutdown. The
                    // first release has no teardown path, so retain them for the
                    // lifetime of the kernel.
                    let _ = alloc::boxed::Box::leak(alloc::boxed::Box::new(resources));
                    ControllerState::Reset
                }
                Err(InitError::Timeout(stage)) => {
                    crate::println!(
                        "[WARN] xhci: {:02x}:{:02x}.{} initialization failed at {}",
                        bdf.bus,
                        bdf.device,
                        bdf.function,
                        stage
                    );
                    ControllerState::Failed
                }
                Err(InitError::Allocation) => {
                    crate::println!(
                        "[WARN] xhci: {:02x}:{:02x}.{} initialization failed at DMA allocation",
                        bdf.bus,
                        bdf.device,
                        bdf.function
                    );
                    ControllerState::Failed
                }
                Err(InitError::Command(code)) => {
                    crate::println!(
                        "[WARN] xhci: {:02x}:{:02x}.{} command completion failed: {:?}",
                        bdf.bus,
                        bdf.device,
                        bdf.function,
                        code
                    );
                    ControllerState::Failed
                }
            }
        }
        Err(ResetError::Timeout(stage)) => {
            crate::println!(
                "[WARN] xhci: {:02x}:{:02x}.{} reset failed at {}",
                bdf.bus,
                bdf.device,
                bdf.function,
                stage
            );
            ControllerState::Failed
        }
    }
}

/// Discover all xHCI PCI functions. Missing or broken hardware is non-fatal.
pub fn init() {
    let mut count = 0;
    doit(|bus, device, function, config| {
        if config.class_code == 0x0c && config.subclass == 0x03 && config.prog_if == 0x30 {
            count += 1;
            let _ = discover_one(
                Bdf {
                    bus,
                    device,
                    function,
                },
                config,
            );
        }
    });
    crate::println!("[INFO] xhci: discovered {} controller(s)", count);
}

/// Run the controller lifecycle's pure bounded-wait checks in kernel context.
pub fn test() {
    assert!(!wait_for(|| false, 3));
    let mut n = 0;
    assert!(wait_for(
        || {
            n += 1;
            n == 2
        },
        3
    ));
    let completion = Trb {
        parameter: 0x4000,
        status: 1 << 24,
        control: TRB_TYPE_COMMAND_COMPLETION | TRB_CYCLE,
    };
    assert_eq!(
        command_completion(completion, 0x4000),
        Some(CompletionCode::Success)
    );
    assert_eq!(command_completion(completion, 0x4010), None);
    assert_eq!(
        command_completion(
            Trb {
                control: TRB_CYCLE,
                ..completion
            },
            0x4000
        ),
        None
    );
}
