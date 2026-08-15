//! PCI discovery and the bounded xHCI controller reset sequence.

use super::bulk::{self, BulkCompletion};
use super::hid::{KeyboardState, MouseState};
use super::msc;
use super::regs;
use super::trb::{
    CompletionCode, TRB_CHAIN, TRB_CYCLE, TRB_TC, TRB_TYPE_COMMAND_COMPLETION, TRB_TYPE_LINK,
    TRB_TYPE_MASK, TRB_TYPE_NORMAL, Trb, port_status_change_event,
};
use super::usb::{
    EndpointDescriptor, HidBootEndpoint, MscBotInterface, PortRecord, PortState, RootPortState,
    SetupRequest, SupportedProtocol, get_descriptor, parse_configuration, parse_msc_bot_interface,
    set_configuration, set_idle, supported_protocol, usb2_max_packet,
};
use crate::mm::dma::{DmaBuffer, MmioMapping};
use crate::pcie::enumrate::{Bdf, MemoryBar, PCIConfigSpace, decode_memory_bar, doit};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};

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
const POLL_EVENT_BUDGET: usize = 32;
const ROOT_PORT_WORK_BUDGET: usize = 2;
const ROOT_PORT_RESCAN_INTERVAL: u64 = 256;
const PCI_CAP_ID_MSI: u8 = 0x05;
const MSI_CONTROL_ENABLE: u16 = 1;
const MSI_CONTROL_MME_MASK: u16 = 0b111 << 4;
const MSI_CONTROL_64BIT: u16 = 1 << 7;

const MAX_CONTROLLERS: usize = 4;

// Controllers are registered during single-threaded boot before MSI is
// enabled.  Atomic slots let the interrupt handler inspect them without
// taking a lock that an interrupted poll() could already hold.
static CONTROLLERS: [AtomicUsize; MAX_CONTROLLERS] =
    [const { AtomicUsize::new(0) }; MAX_CONTROLLERS];

fn msi_data_offset(capability_offset: usize, control: u16) -> usize {
    // The message data follows the 32-bit address, and an enabled 64-bit
    // address capability contributes an additional DWORD.
    if control & MSI_CONTROL_64BIT != 0 {
        capability_offset + 12
    } else {
        capability_offset + 8
    }
}

fn register_controller(controller: *mut ControllerResources) -> bool {
    for slot in &CONTROLLERS {
        if slot
            .compare_exchange(0, controller as usize, Ordering::Release, Ordering::Relaxed)
            .is_ok()
        {
            return true;
        }
    }
    false
}

struct RootDevice {
    port: u8,
    generation: u64,
    state: PortState,
    _input_context: DmaBuffer,
    _output_context: DmaBuffer,
    _ep0_ring: DmaBuffer,
    _device_descriptor: DmaBuffer,
    _configuration_descriptor: DmaBuffer,
    kind: DeviceKind,
}

#[allow(dead_code)] // MSC rings stay owned while the configured device is active.
enum DeviceKind {
    Hid(HidResources),
    Msc(MscResources),
}

struct HidResources {
    interrupt_ring: DmaBuffer,
    report: DmaBuffer,
    endpoint: HidBootEndpoint,
    producer: usize,
    cycle: bool,
    input: HidInput,
}

struct MscResources {
    _bulk_in_ring: DmaBuffer,
    _bulk_out_ring: DmaBuffer,
}

enum HidInput {
    Keyboard(KeyboardState),
    Mouse(MouseState),
}

impl RootDevice {
    fn slot(&self) -> u8 {
        match self.state {
            PortState::Addressed { slot, .. } => slot,
            _ => 0,
        }
    }
}

fn endpoint_id(address: u8) -> u8 {
    ((address & 0x0f) * 2) + u8::from(address & 0x80 != 0)
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
    root_ports: alloc::vec::Vec<PortRecord>,
    protocols: alloc::vec::Vec<SupportedProtocol>,
    interrupt_pending: AtomicBool,
    interrupt_count: AtomicUsize,
    msi_vector: Option<u8>,
    logged_interrupt_count: usize,
    poll_ticks: u64,
    healthy: bool,
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
            root_ports: alloc::vec::Vec::new(),
            protocols: alloc::vec::Vec::new(),
            interrupt_pending: AtomicBool::new(false),
            interrupt_count: AtomicUsize::new(0),
            msi_vector: None,
            logged_interrupt_count: 0,
            poll_ticks: 0,
            healthy: true,
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
                let iman = register.read32(self.runtime + regs::RT_INTR0 + regs::IMAN);
                register.write32(
                    self.runtime + regs::RT_INTR0 + regs::IMAN,
                    (iman & regs::IMAN_IE) | regs::IMAN_IP,
                );
                register.write32(self.op + regs::USBSTS, regs::USBSTS_EINT);
            }
            if let Some(change) = port_status_change_event(trb) {
                self.handle_port_status_change(register, change.port_id);
                continue;
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
                let iman = register.read32(self.runtime + regs::RT_INTR0 + regs::IMAN);
                register.write32(
                    self.runtime + regs::RT_INTR0 + regs::IMAN,
                    (iman & regs::IMAN_IE) | regs::IMAN_IP,
                );
                register.write32(self.op + regs::USBSTS, regs::USBSTS_EINT);
            }
            if let Some(change) = port_status_change_event(event) {
                self.handle_port_status_change(register, change.port_id);
                continue;
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

    fn acknowledge_event(&mut self, register: regs::RegisterBlock) {
        self.event_consumer = (self.event_consumer + 1) % EVENT_RING_ENTRIES;
        if self.event_consumer == 0 {
            self.event_cycle = !self.event_cycle;
        }
        self.event_count += 1;
        unsafe {
            register.write64(
                self.runtime + regs::RT_INTR0 + regs::ERDP,
                self.event_ring.physical_address() + (self.event_consumer * 16) as u64
                    | regs::ERDP_EHB,
            );
            let iman = register.read32(self.runtime + regs::RT_INTR0 + regs::IMAN);
            register.write32(
                self.runtime + regs::RT_INTR0 + regs::IMAN,
                (iman & regs::IMAN_IE) | regs::IMAN_IP,
            );
            register.write32(self.op + regs::USBSTS, regs::USBSTS_EINT);
        }
    }

    fn queue_interrupt_in(
        register: regs::RegisterBlock,
        doorbell: usize,
        slot: u8,
        hid: &mut HidResources,
    ) {
        let index = hid.producer;
        let trb = Trb {
            parameter: hid.report.physical_address(),
            status: hid.endpoint.max_packet_size as u32,
            control: TRB_TYPE_NORMAL | (1 << 5) | hid.cycle as u32,
        };
        unsafe {
            core::ptr::write_volatile((hid.interrupt_ring.as_ptr() as *mut Trb).add(index), trb);
        }
        fence(Ordering::SeqCst);
        let endpoint_id = endpoint_id(hid.endpoint.endpoint_address);
        unsafe { register.write32(doorbell + slot as usize * 4, endpoint_id as u32) };
        hid.producer += 1;
        if hid.producer == TRANSFER_RING_ENTRIES {
            hid.producer = 0;
            hid.cycle = !hid.cycle;
            unsafe {
                core::ptr::write_volatile(
                    (hid.interrupt_ring.as_ptr() as *mut Trb).add(TRANSFER_RING_ENTRIES),
                    Trb {
                        parameter: hid.interrupt_ring.physical_address(),
                        control: TRB_TYPE_LINK | TRB_TC | hid.cycle as u32,
                        ..Trb::default()
                    },
                );
            }
        }
    }

    fn poll(&mut self, budget: usize) {
        if !self.healthy {
            return;
        }
        let register = unsafe { regs::RegisterBlock::new(self._mapping.as_ptr()) };
        self.poll_ticks = self.poll_ticks.wrapping_add(1);
        if self.interrupt_pending.swap(false, Ordering::AcqRel) {
            let count = self.interrupt_count.load(Ordering::Acquire);
            if self.logged_interrupt_count == 0 {
                crate::println!(
                    "[INFO] xhci: MSI vector {} delivered event count {}",
                    self.msi_vector.unwrap_or(0),
                    count
                );
            }
            self.logged_interrupt_count = count;
        }
        for _ in 0..budget {
            let event = unsafe {
                core::ptr::read_volatile(
                    self.event_ring.as_ptr().add(self.event_consumer * 16) as *const Trb
                )
            };
            if (event.control & TRB_CYCLE != 0) != self.event_cycle {
                break;
            }
            self.acknowledge_event(register);
            if let Some(change) = port_status_change_event(event) {
                self.handle_port_status_change(register, change.port_id);
                continue;
            }
            if event.control & TRB_TYPE_MASK != super::trb::TRB_TYPE_TRANSFER_EVENT {
                continue;
            }
            let slot = (event.control >> 24) as u8;
            let endpoint = ((event.control >> 16) & 0x1f) as u8;
            let generation = self
                .ports
                .iter()
                .find(|device| device.slot() == slot)
                .and_then(|device| {
                    self.root_ports.iter().find_map(|record| {
                        (record.port == device.port && record.state == RootPortState::Active)
                            .then_some(record.generation)
                    })
                });
            let Some(device) = self.ports.iter_mut().find(|device| {
                device.slot() == slot
                    && generation == Some(device.generation)
                    && matches!(&device.kind, DeviceKind::Hid(hid) if endpoint_id(hid.endpoint.endpoint_address) == endpoint)
            }) else {
                continue;
            };
            let DeviceKind::Hid(hid) = &mut device.kind else {
                continue;
            };
            let completion = CompletionCode::from_status(event.status);
            if !matches!(
                completion,
                CompletionCode::Success | CompletionCode::ShortPacket
            ) {
                crate::println!(
                    "[WARN] xhci: transfer failed slot {}, endpoint {}, code {:?}",
                    slot,
                    endpoint,
                    completion
                );
                Self::queue_interrupt_in(register, self.doorbell, slot, hid);
                continue;
            }
            let residual = (event.status & 0x00ff_ffff) as usize;
            let max_packet = hid.endpoint.max_packet_size as usize;
            if residual <= max_packet {
                let report_len = max_packet - residual;
                let report =
                    unsafe { core::slice::from_raw_parts(hid.report.as_ptr(), report_len) };
                match &mut hid.input {
                    HidInput::Keyboard(state) => state.submit_report(report),
                    HidInput::Mouse(state) => state.submit_report(report),
                }
            }
            Self::queue_interrupt_in(register, self.doorbell, slot, hid);
        }
        self.service_root_ports(register);
    }

    fn enable_msi(&mut self, config: &PCIConfigSpace, bdf: Bdf) {
        let Some(capability) = config.capabilities().find(|cap| cap.id == PCI_CAP_ID_MSI) else {
            crate::println!(
                "[INFO] xhci: {:02x}:{:02x}.{} has no MSI capability; using polling fallback",
                bdf.bus,
                bdf.device,
                bdf.function
            );
            return;
        };
        let control = config.read_u16(capability.offset as usize + 2);
        let data_offset = msi_data_offset(capability.offset as usize, control);
        let destination = crate::apic::local::lapic_id() as u64;
        let address = 0xfee0_0000u64 | (destination << 12);
        let vector = crate::int::XHCI_MSI_VECTOR;
        unsafe {
            config.write_u32(capability.offset as usize + 4, address as u32);
            if control & MSI_CONTROL_64BIT != 0 {
                config.write_u32(capability.offset as usize + 8, (address >> 32) as u32);
            }
            config.write_u16(data_offset, vector as u16);
            // Use one message, retain capability-defined read-only bits, and
            // enable MSI only after the IDT vector and controller slot exist.
            config.write_u16(
                capability.offset as usize + 2,
                (control & !MSI_CONTROL_MME_MASK) | MSI_CONTROL_ENABLE,
            );
            // The xHCI global interrupt gate is independent from IMAN.IE and
            // PCI MSI enable.  Leave it clear for polling fallback, then open
            // it only after the MSI message and IDT vector are ready.
            let register = regs::RegisterBlock::new(self._mapping.as_ptr());
            register.write32(
                self.op + regs::USBCMD,
                register.read32(self.op + regs::USBCMD) | regs::USBCMD_INTE,
            );
        }
        self.msi_vector = Some(vector);
        crate::println!(
            "[INFO] xhci: {:02x}:{:02x}.{} MSI vector {:#x} configured",
            bdf.bus,
            bdf.device,
            bdf.function,
            vector
        );
    }

    fn acknowledge_interrupt(&self) -> bool {
        let register = unsafe { regs::RegisterBlock::new(self._mapping.as_ptr()) };
        let status = unsafe { register.read32(self.op + regs::USBSTS) };
        let iman = unsafe { register.read32(self.runtime + regs::RT_INTR0 + regs::IMAN) };
        if status & regs::USBSTS_EINT == 0 && iman & regs::IMAN_IP == 0 {
            return false;
        }
        unsafe {
            if status & regs::USBSTS_EINT != 0 {
                register.write32(self.op + regs::USBSTS, regs::USBSTS_EINT);
            }
            if iman & regs::IMAN_IP != 0 {
                register.write32(
                    self.runtime + regs::RT_INTR0 + regs::IMAN,
                    (iman & regs::IMAN_IE) | regs::IMAN_IP,
                );
            }
        }
        self.interrupt_count.fetch_add(1, Ordering::Release);
        self.interrupt_pending.store(true, Ordering::Release);
        true
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
        self.protocols = protocols;
        self.root_ports = self
            .protocols
            .iter()
            .filter(|protocol| protocol.usb2)
            .flat_map(|protocol| {
                (protocol.port_start..protocol.port_start.saturating_add(protocol.port_count))
                    .map(move |port| PortRecord::new(port, *protocol, 0))
            })
            .collect();
        for protocol in self.protocols.clone() {
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
                if let Some(record) = self
                    .root_ports
                    .iter_mut()
                    .find(|record| record.port == port)
                {
                    record.portsc = portsc & !regs::PORTSC_CHANGE_MASK;
                    record.mark_stage(RootPortState::Enumerating);
                }
                if portsc & regs::PORTSC_CCS == 0 {
                    if let Some(record) = self
                        .root_ports
                        .iter_mut()
                        .find(|record| record.port == port)
                    {
                        record.mark_stage(RootPortState::Disconnected);
                    }
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
                let generation = self
                    .root_ports
                    .iter()
                    .find(|record| record.port == port)
                    .map(|record| record.generation)
                    .unwrap_or(0);
                match self.address_port(register, port, speed, generation) {
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
                        if let Some(record) = self
                            .root_ports
                            .iter_mut()
                            .find(|record| record.port == port)
                        {
                            record.mark_active();
                        }
                    }
                    Err(error) => {
                        if let Some(record) = self
                            .root_ports
                            .iter_mut()
                            .find(|record| record.port == port)
                        {
                            record.mark_failed(0);
                        }
                        crate::println!(
                            "[WARN] xhci: port {} enumeration failed: {:?}",
                            port,
                            error
                        )
                    }
                }
            }
        }
    }

    fn handle_port_status_change(&mut self, register: regs::RegisterBlock, port: u8) {
        if port == 0 {
            return;
        }
        let offset = self.op + regs::PORTSC + (port as usize - 1) * 0x10;
        let value = unsafe { register.read32(offset) };
        let changes = value & regs::PORTSC_CHANGE_MASK;
        if changes != 0 {
            // PORTSC change bits are W1C; acknowledge all known causes at once.
            unsafe { register.write32(offset, (value & !regs::PORTSC_CHANGE_MASK) | changes) };
        }
        let Some(record) = self
            .root_ports
            .iter_mut()
            .find(|record| record.port == port)
        else {
            return;
        };
        if value & regs::PORTSC_CCS != 0
            && matches!(record.state, RootPortState::Active)
            && self.ports.iter().any(|device| device.port == port)
        {
            record.portsc = value & !regs::PORTSC_CHANGE_MASK;
            return;
        }
        record.observe(value, self.poll_ticks);
    }

    fn port_offset(&self, port: u8) -> usize {
        self.op + regs::PORTSC + (port as usize - 1) * 0x10
    }

    fn observe_port(&mut self, register: regs::RegisterBlock, port: u8) {
        let value = unsafe { register.read32(self.port_offset(port)) };
        let changes = value & regs::PORTSC_CHANGE_MASK;
        if changes != 0 {
            unsafe {
                register.write32(
                    self.port_offset(port),
                    (value & !regs::PORTSC_CHANGE_MASK) | changes,
                )
            };
        }
        if let Some(record) = self
            .root_ports
            .iter_mut()
            .find(|record| record.port == port)
        {
            if value & regs::PORTSC_CCS != 0
                && matches!(record.state, RootPortState::Active)
                && self.ports.iter().any(|device| device.port == port)
            {
                record.portsc = value & !regs::PORTSC_CHANGE_MASK;
                return;
            }
            record.observe(value, self.poll_ticks);
        }
    }

    fn service_root_ports(&mut self, register: regs::RegisterBlock) {
        if self.poll_ticks % ROOT_PORT_RESCAN_INTERVAL == 0 {
            let ports: alloc::vec::Vec<u8> =
                self.root_ports.iter().map(|record| record.port).collect();
            for port in ports {
                self.observe_port(register, port);
            }
        }

        for _ in 0..ROOT_PORT_WORK_BUDGET {
            let next = self
                .root_ports
                .iter()
                .find_map(|record| match record.state {
                    RootPortState::Removing => Some((record.port, record.generation, false)),
                    RootPortState::Debouncing { until } if self.poll_ticks >= until => {
                        Some((record.port, record.generation, true))
                    }
                    RootPortState::Failed { .. } if record.retry_due(self.poll_ticks) => {
                        Some((record.port, record.generation, true))
                    }
                    _ => None,
                });
            let Some((port, generation, connect)) = next else {
                break;
            };
            if !connect {
                self.remove_port(register, port, generation);
                continue;
            }
            self.enumerate_port(register, port, generation);
        }
    }

    fn enumerate_port(&mut self, register: regs::RegisterBlock, port: u8, generation: u64) {
        let offset = self.port_offset(port);
        let portsc = unsafe { register.read32(offset) };
        if portsc & regs::PORTSC_CCS == 0 {
            self.observe_port(register, port);
            return;
        }
        let speed = ((portsc >> 10) & 0xf) as u8;
        if speed >= 4 {
            crate::println!(
                "[INFO] xhci: port {} connected at SuperSpeed {}, skipping",
                port,
                speed
            );
            if let Some(record) = self
                .root_ports
                .iter_mut()
                .find(|record| record.port == port)
            {
                record.mark_failed(self.poll_ticks);
            }
            return;
        }
        if let Some(record) = self
            .root_ports
            .iter_mut()
            .find(|record| record.port == port)
        {
            if !record.generation_matches(generation) {
                return;
            }
            record.mark_stage(RootPortState::Resetting);
        }
        crate::println!("[INFO] xhci: port {} connected, resetting", port);
        if !self.reset_port(register, offset) {
            crate::println!("[WARN] xhci: port {} reset failed", port);
            if let Some(record) = self
                .root_ports
                .iter_mut()
                .find(|record| record.port == port)
            {
                record.mark_failed(self.poll_ticks);
            }
            return;
        }
        let portsc = unsafe { register.read32(offset) };
        if portsc & regs::PORTSC_CCS == 0 {
            self.observe_port(register, port);
            return;
        }
        let speed = ((portsc >> 10) & 0xf) as u8;
        if let Some(record) = self
            .root_ports
            .iter_mut()
            .find(|record| record.port == port)
        {
            if !record.generation_matches(generation) {
                return;
            }
            record.mark_stage(RootPortState::Enumerating);
        }
        match self.address_port(register, port, speed, generation) {
            Ok(device) => {
                if self.port_is_current(port, generation) {
                    crate::println!(
                        "[INFO] xhci: port {} add slot {} generation {}",
                        port,
                        device.slot(),
                        generation
                    );
                    self.ports.push(device);
                    if let Some(record) = self
                        .root_ports
                        .iter_mut()
                        .find(|record| record.port == port)
                    {
                        record.mark_active();
                    }
                } else {
                    let slot = device.slot();
                    if self.disable_slot(register, slot) {
                        unsafe {
                            core::ptr::write_volatile(
                                (self.dcbaa.as_ptr() as *mut u64).add(slot as usize),
                                0,
                            )
                        };
                        fence(Ordering::SeqCst);
                    } else {
                        // Disable Slot did not establish that xHCI has stopped
                        // DMA. Keep every buffer reachable for the remainder
                        // of the isolated controller's lifetime.
                        self.ports.push(device);
                    }
                }
            }
            Err(error) => {
                crate::println!("[WARN] xhci: port {} enumeration failed: {:?}", port, error);
                if let Some(record) = self
                    .root_ports
                    .iter_mut()
                    .find(|record| record.port == port)
                {
                    record.mark_failed(self.poll_ticks);
                }
            }
        }
    }

    fn port_is_current(&self, port: u8, generation: u64) -> bool {
        let value = unsafe {
            regs::RegisterBlock::new(self._mapping.as_ptr()).read32(self.port_offset(port))
        };
        value & regs::PORTSC_CCS != 0
            && self
                .root_ports
                .iter()
                .any(|record| record.port == port && record.generation_matches(generation))
    }

    fn disable_slot(&mut self, register: regs::RegisterBlock, slot: u8) -> bool {
        match self.submit_command(
            register,
            Trb {
                control: regs::TRB_TYPE_DISABLE_SLOT | (slot as u32) << 24,
                ..Trb::default()
            },
        ) {
            Ok(_) => true,
            Err(error) => {
                crate::println!(
                    "[WARN] xhci: Disable Slot {} failed: {:?}; retaining DMA and isolating controller",
                    slot,
                    error
                );
                self.healthy = false;
                false
            }
        }
    }

    fn remove_port(&mut self, register: regs::RegisterBlock, port: u8, generation: u64) {
        let Some(index) = self.ports.iter().position(|device| device.port == port) else {
            if let Some(record) = self
                .root_ports
                .iter_mut()
                .find(|record| record.port == port && record.generation_matches(generation))
            {
                record.mark_stage(RootPortState::Disconnected);
            }
            return;
        };
        let slot = self.ports[index].slot();
        if !self.disable_slot(register, slot) {
            return;
        }
        unsafe {
            core::ptr::write_volatile((self.dcbaa.as_ptr() as *mut u64).add(slot as usize), 0)
        };
        fence(Ordering::SeqCst);
        self.ports.remove(index);
        crate::println!(
            "[INFO] xhci: port {} remove slot {} generation {} reclaimed",
            port,
            slot,
            generation
        );
        if let Some(record) = self
            .root_ports
            .iter_mut()
            .find(|record| record.port == port && record.generation_matches(generation))
        {
            record.mark_stage(RootPortState::Disconnected);
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
        generation: u64,
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
        let input = match DmaBuffer::new(context_size * 33, 64) {
            Ok(buffer) => buffer,
            Err(_) => {
                self.release_failed_slot(register, slot);
                return Err(InitError::Allocation);
            }
        };
        let output = match DmaBuffer::new(context_size * 32, 64) {
            Ok(buffer) => buffer,
            Err(_) => {
                self.release_failed_slot(register, slot);
                return Err(InitError::Allocation);
            }
        };
        let ep0 = match DmaBuffer::new(4096, 64) {
            Ok(buffer) => buffer,
            Err(_) => {
                self.release_failed_slot(register, slot);
                return Err(InitError::Allocation);
            }
        };
        let descriptor = match DmaBuffer::new(18, 64) {
            Ok(buffer) => buffer,
            Err(_) => {
                self.release_failed_slot(register, slot);
                return Err(InitError::Allocation);
            }
        };
        (|| -> Result<RootDevice, InitError> {
            macro_rules! fail_published {
                ($error:expr) => {{
                    self.release_or_retain_slot_resources(
                        register,
                        slot,
                        [input, output, ep0, descriptor],
                    );
                    return Err($error);
                }};
            }
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
                fail_published!(error);
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
                fail_published!(error);
            }
            let config_header = match DmaBuffer::new(9, 64) {
                Ok(buffer) => buffer,
                Err(_) => fail_published!(InitError::Allocation),
            };
            if let Err(error) = self.control_transfer(
                register,
                slot,
                &ep0,
                &mut ep0_producer,
                get_descriptor(2, 9),
                Some(&config_header),
            ) {
                fail_published!(error);
            }
            let header = unsafe { core::slice::from_raw_parts(config_header.as_ptr(), 9) };
            let total = u16::from_le_bytes([header[2], header[3]]) as usize;
            if total < 9 || total > 4096 {
                fail_published!(InitError::Command(CompletionCode::TrbError));
            }
            let configuration = match DmaBuffer::new(total, 64) {
                Ok(buffer) => buffer,
                Err(_) => fail_published!(InitError::Allocation),
            };
            if let Err(error) = self.control_transfer(
                register,
                slot,
                &ep0,
                &mut ep0_producer,
                get_descriptor(2, total as u16),
                Some(&configuration),
            ) {
                fail_published!(error);
            }
            let config_bytes =
                unsafe { core::slice::from_raw_parts(configuration.as_ptr(), total) };
            let endpoint = match parse_configuration(config_bytes) {
                Ok(endpoint) => endpoint,
                Err(_) => {
                    let msc_endpoint = match parse_msc_bot_interface(config_bytes) {
                        Ok(endpoint) => endpoint,
                        Err(_) => fail_published!(InitError::Command(CompletionCode::TrbError)),
                    };
                    let msc = match self.configure_and_probe_msc(
                        register,
                        slot,
                        &ep0,
                        &mut ep0_producer,
                        &input,
                        msc_endpoint,
                    ) {
                        Ok(msc) => msc,
                        Err(error) => fail_published!(error),
                    };
                    return Ok(RootDevice {
                        port,
                        generation,
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
                        kind: DeviceKind::Msc(msc),
                    });
                }
            };
            log_hid_endpoint(slot, endpoint);
            if let Err(error) = self.control_transfer(
                register,
                slot,
                &ep0,
                &mut ep0_producer,
                set_configuration(endpoint.configuration_value),
                None,
            ) {
                fail_published!(error);
            }
            if let Err(error) = self.control_transfer(
                register,
                slot,
                &ep0,
                &mut ep0_producer,
                set_idle(endpoint.interface_number),
                None,
            ) {
                fail_published!(error);
            }
            let interrupt_ring = match DmaBuffer::new(4096, 64) {
                Ok(buffer) => buffer,
                Err(_) => fail_published!(InitError::Allocation),
            };
            if let Err(error) =
                self.configure_endpoint(register, slot, &input, endpoint, &interrupt_ring)
            {
                self.release_or_retain_slot_resources(
                    register,
                    slot,
                    [input, output, ep0, descriptor, interrupt_ring],
                );
                return Err(error);
            }
            crate::println!(
                "[INFO] xhci: HID endpoint configured slot {}, endpoint {:#04x}, interval {}, max packet {}, DCS 1",
                slot,
                endpoint.endpoint_address,
                endpoint.interval,
                endpoint.max_packet_size
            );
            let report = match DmaBuffer::new(endpoint.max_packet_size as usize, 64) {
                Ok(buffer) => buffer,
                Err(_) => {
                    self.release_or_retain_slot_resources(
                        register,
                        slot,
                        [input, output, ep0, descriptor, interrupt_ring],
                    );
                    return Err(InitError::Allocation);
                }
            };
            let mut device = RootDevice {
                port,
                generation,
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
                kind: DeviceKind::Hid(HidResources {
                    interrupt_ring,
                    report,
                    endpoint,
                    producer: 0,
                    cycle: true,
                    input: if endpoint.protocol == 1 {
                        HidInput::Keyboard(KeyboardState::new())
                    } else {
                        HidInput::Mouse(MouseState::new())
                    },
                }),
            };
            if let DeviceKind::Hid(hid) = &mut device.kind {
                Self::queue_interrupt_in(register, self.doorbell, slot, hid);
            }
            Ok(device)
        })()
    }

    fn release_or_retain_slot_resources<const N: usize>(
        &mut self,
        register: regs::RegisterBlock,
        slot: u8,
        resources: [DmaBuffer; N],
    ) {
        if self.disable_slot(register, slot) {
            unsafe {
                core::ptr::write_volatile((self.dcbaa.as_ptr() as *mut u64).add(slot as usize), 0)
            };
            fence(Ordering::SeqCst);
        } else {
            for resource in resources {
                core::mem::forget(resource);
            }
        }
    }

    fn release_failed_slot(&mut self, register: regs::RegisterBlock, slot: u8) {
        if self.disable_slot(register, slot) {
            unsafe {
                core::ptr::write_volatile((self.dcbaa.as_ptr() as *mut u64).add(slot as usize), 0)
            };
            fence(Ordering::SeqCst);
            crate::println!(
                "[INFO] xhci: slot {} released after failed enumeration",
                slot
            );
        }
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

    /// Submit exactly one bulk Normal TRB and wait for its matching event.
    /// The caller owns a distinct endpoint ring and DMA buffer, so no HID TD
    /// can be overwritten or mistaken for this completion.
    #[allow(dead_code)]
    fn bulk_transfer(
        &mut self,
        register: regs::RegisterBlock,
        slot: u8,
        endpoint_address: u8,
        ring: &DmaBuffer,
        producer: &mut usize,
        cycle: &mut bool,
        buffer: &DmaBuffer,
        len: usize,
    ) -> Result<BulkCompletion, InitError> {
        if *producer >= TRANSFER_RING_ENTRIES || len > buffer.len() {
            return Err(InitError::Command(CompletionCode::RingOverrun));
        }
        let in_direction = endpoint_address & 0x80 != 0;
        let trb = bulk::normal_trb(buffer.physical_address(), len, in_direction, *cycle)
            .map_err(|_| InitError::Command(CompletionCode::TrbError))?;
        let trb_address = ring.physical_address() + (*producer * 16) as u64;
        unsafe { core::ptr::write_volatile((ring.as_ptr() as *mut Trb).add(*producer), trb) };
        fence(Ordering::SeqCst);
        unsafe {
            register.write32(
                self.doorbell + slot as usize * 4,
                endpoint_id(endpoint_address) as u32,
            )
        };
        let (code, residual) = self.poll_bulk_completion(register, trb_address)?;
        *producer += 1;
        if *producer == TRANSFER_RING_ENTRIES {
            *producer = 0;
            *cycle = !*cycle;
            unsafe {
                core::ptr::write_volatile(
                    (ring.as_ptr() as *mut Trb).add(TRANSFER_RING_ENTRIES),
                    Trb {
                        parameter: ring.physical_address(),
                        control: TRB_TYPE_LINK | TRB_TC | *cycle as u32,
                        ..Trb::default()
                    },
                );
            }
        }
        bulk::completion(code, len, residual, in_direction).map_err(|error| match error {
            bulk::BulkError::Completion(code) => InitError::Command(code),
            _ => InitError::Command(CompletionCode::TrbError),
        })
    }

    #[allow(dead_code)]
    fn poll_bulk_completion(
        &mut self,
        register: regs::RegisterBlock,
        trb_address: u64,
    ) -> Result<(CompletionCode, usize), InitError> {
        for poll in 0..TRANSFER_POLL_LIMIT {
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
            self.acknowledge_event(register);
            if let Some(change) = port_status_change_event(event) {
                self.handle_port_status_change(register, change.port_id);
                continue;
            }
            if event.control & TRB_TYPE_MASK == super::trb::TRB_TYPE_TRANSFER_EVENT
                && event.parameter == trb_address
            {
                return Ok((
                    CompletionCode::from_status(event.status),
                    (event.status & 0x00ff_ffff) as usize,
                ));
            }
        }
        Err(InitError::Timeout("bulk transfer"))
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
        let endpoint_id = endpoint_id(endpoint.endpoint_address) as usize;
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

    /// Configure one Bulk IN or OUT endpoint with an independently-owned
    /// transfer ring.  BOT configures its pair with separate calls/rings.
    #[allow(dead_code)]
    fn configure_bulk_endpoint(
        &mut self,
        register: regs::RegisterBlock,
        slot: u8,
        input: &DmaBuffer,
        endpoint: EndpointDescriptor,
        ring: &DmaBuffer,
    ) -> Result<(), InitError> {
        let context_size = if self.context_64 { 64 } else { 32 };
        let endpoint_id = endpoint_id(endpoint.address) as usize;
        if endpoint_id == 0
            || endpoint_id >= 32
            || endpoint.attributes != 0x02
            || endpoint.max_packet_size == 0
        {
            return Err(InitError::Command(CompletionCode::TrbError));
        }
        unsafe {
            let control = input.as_ptr().add(4) as *mut u32;
            core::ptr::write_volatile(control, 1 | 1 << endpoint_id);
            let slot_context = input.as_ptr().add(context_size) as *mut u32;
            let old = core::ptr::read_volatile(slot_context);
            core::ptr::write_volatile(
                slot_context,
                (old & !(0x1f << 27)) | (endpoint_id as u32) << 27,
            );
            let context = input.as_ptr().add(context_size * (endpoint_id + 1)) as *mut u32;
            // Bulk endpoint type: OUT=2, IN=6. CErr=3.
            let endpoint_type = if endpoint.address & 0x80 != 0 { 6 } else { 2 };
            core::ptr::write_volatile(context, 0);
            core::ptr::write_volatile(
                context.add(1),
                (endpoint.max_packet_size as u32) << 16 | endpoint_type << 3 | 3 << 1,
            );
            core::ptr::write_volatile(context.add(2) as *mut u64, ring.physical_address() | 1);
            core::ptr::write_volatile(
                (ring.as_ptr() as *mut Trb).add(TRANSFER_RING_ENTRIES),
                Trb {
                    parameter: ring.physical_address(),
                    control: TRB_TYPE_LINK | TRB_TC | TRB_CYCLE,
                    ..Trb::default()
                },
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

    fn configure_and_probe_msc(
        &mut self,
        register: regs::RegisterBlock,
        slot: u8,
        ep0: &DmaBuffer,
        ep0_producer: &mut usize,
        input: &DmaBuffer,
        endpoint: MscBotInterface,
    ) -> Result<MscResources, InitError> {
        self.control_transfer(
            register,
            slot,
            ep0,
            ep0_producer,
            set_configuration(endpoint.configuration_value),
            None,
        )?;
        let bulk_in_ring = DmaBuffer::new(4096, 64).map_err(|_| InitError::Allocation)?;
        let bulk_out_ring = DmaBuffer::new(4096, 64).map_err(|_| InitError::Allocation)?;
        self.configure_bulk_endpoint(register, slot, input, endpoint.bulk_out, &bulk_out_ring)?;
        self.configure_bulk_endpoint(register, slot, input, endpoint.bulk_in, &bulk_in_ring)?;
        let max_lun = DmaBuffer::new(1, 64).map_err(|_| InitError::Allocation)?;
        match self.control_transfer(
            register,
            slot,
            ep0,
            ep0_producer,
            msc::get_max_lun(endpoint.interface_number),
            Some(&max_lun),
        ) {
            Ok(()) => {
                if unsafe { core::ptr::read_volatile(max_lun.as_ptr()) } != 0 {
                    return Err(InitError::Command(CompletionCode::TrbError));
                }
            }
            Err(InitError::Command(CompletionCode::StallError)) => {}
            Err(error) => return Err(error),
        }
        let cbw = DmaBuffer::new(msc::CBW_LEN, 64).map_err(|_| InitError::Allocation)?;
        let csw = DmaBuffer::new(msc::CSW_LEN, 64).map_err(|_| InitError::Allocation)?;
        let data = DmaBuffer::new(64, 64).map_err(|_| InitError::Allocation)?;
        let mut in_producer = 0;
        let mut out_producer = 0;
        let mut in_cycle = true;
        let mut out_cycle = true;
        self.msc_command(
            register,
            slot,
            endpoint,
            &bulk_in_ring,
            &bulk_out_ring,
            &mut in_producer,
            &mut out_producer,
            &mut in_cycle,
            &mut out_cycle,
            &cbw,
            &csw,
            &data,
            36,
            &msc::inquiry(),
        )?;
        let inquiry = unsafe { core::slice::from_raw_parts(data.as_ptr(), 36) };
        crate::println!(
            "[INFO] xhci: MSC BOT slot {}, LUN 0, vendor {:?}, product {:?}",
            slot,
            &inquiry[8..16],
            &inquiry[16..32]
        );
        self.msc_command(
            register,
            slot,
            endpoint,
            &bulk_in_ring,
            &bulk_out_ring,
            &mut in_producer,
            &mut out_producer,
            &mut in_cycle,
            &mut out_cycle,
            &cbw,
            &csw,
            &data,
            0,
            &msc::test_unit_ready(),
        )?;
        self.msc_command(
            register,
            slot,
            endpoint,
            &bulk_in_ring,
            &bulk_out_ring,
            &mut in_producer,
            &mut out_producer,
            &mut in_cycle,
            &mut out_cycle,
            &cbw,
            &csw,
            &data,
            8,
            &msc::read_capacity10(),
        )?;
        let capacity =
            msc::parse_capacity10(unsafe { core::slice::from_raw_parts(data.as_ptr(), 8) })
                .map_err(|_| InitError::Command(CompletionCode::TrbError))?;
        crate::println!(
            "[INFO] xhci: MSC BOT slot {}, LUN 0, capacity {} blocks, block size {}",
            slot,
            capacity.blocks,
            capacity.block_size
        );
        Ok(MscResources {
            _bulk_in_ring: bulk_in_ring,
            _bulk_out_ring: bulk_out_ring,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn msc_command(
        &mut self,
        register: regs::RegisterBlock,
        slot: u8,
        endpoint: MscBotInterface,
        bulk_in_ring: &DmaBuffer,
        bulk_out_ring: &DmaBuffer,
        in_producer: &mut usize,
        out_producer: &mut usize,
        in_cycle: &mut bool,
        out_cycle: &mut bool,
        cbw: &DmaBuffer,
        csw: &DmaBuffer,
        data: &DmaBuffer,
        data_len: usize,
        cdb: &[u8],
    ) -> Result<(), InitError> {
        let tag = (*out_producer as u32).wrapping_add(1);
        let packet = msc::cbw(tag, data_len as u32, msc::DataDirection::In, 0, cdb);
        unsafe { core::ptr::copy_nonoverlapping(packet.as_ptr(), cbw.as_ptr(), packet.len()) };
        self.bulk_transfer(
            register,
            slot,
            endpoint.bulk_out.address,
            bulk_out_ring,
            out_producer,
            out_cycle,
            cbw,
            packet.len(),
        )?;
        if data_len != 0 {
            self.bulk_transfer(
                register,
                slot,
                endpoint.bulk_in.address,
                bulk_in_ring,
                in_producer,
                in_cycle,
                data,
                data_len,
            )?;
        }
        self.bulk_transfer(
            register,
            slot,
            endpoint.bulk_in.address,
            bulk_in_ring,
            in_producer,
            in_cycle,
            csw,
            msc::CSW_LEN,
        )?;
        let status = unsafe { core::slice::from_raw_parts(csw.as_ptr(), msc::CSW_LEN) };
        msc::Csw::parse(status)
            .and_then(|status| status.validate(tag, data_len))
            .map_err(|_| InitError::Command(CompletionCode::TrbError))
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
                    // These resources contain HHDM/MMIO pointers and are not
                    // Send. Keep each controller at a stable kernel-lifetime
                    // address before MSI can expose it to the IDT handler.
                    let controller = alloc::boxed::Box::leak(alloc::boxed::Box::new(resources));
                    if !register_controller(controller) {
                        crate::println!(
                            "[WARN] xhci: {:02x}:{:02x}.{} controller limit reached; using polling without MSI",
                            bdf.bus,
                            bdf.device,
                            bdf.function
                        );
                    } else {
                        controller.enable_msi(config, bdf);
                    }
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

/// Consume a bounded number of HID transfer events for every live controller.
/// This is deliberately called from the kernel idle path rather than an IDT
/// handler, so report decoding and terminal submission never run in interrupt
/// context.
pub fn poll() {
    for controller in &CONTROLLERS {
        let controller = controller.load(Ordering::Acquire);
        if controller != 0 {
            // Entries are leaked only after successful initialization and
            // remain valid until kernel shutdown. poll() is called from the
            // sole idle context; the MSI handler touches only atomics/MMIO.
            unsafe { (&mut *(controller as *mut ControllerResources)).poll(POLL_EVENT_BUDGET) };
        }
    }
}

/// Acknowledge xHCI interrupt causes and defer all event parsing to poll().
/// This is invoked only by the dedicated MSI vector.
pub fn interrupt_handler() {
    for controller in &CONTROLLERS {
        let controller = controller.load(Ordering::Acquire);
        if controller != 0 {
            unsafe { (&*(controller as *const ControllerResources)).acknowledge_interrupt() };
        }
    }
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
    assert_eq!(msi_data_offset(0x40, 0), 0x48);
    assert_eq!(msi_data_offset(0x40, MSI_CONTROL_64BIT), 0x4c);
}
