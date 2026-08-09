//! PCI discovery and the bounded xHCI controller reset sequence.

use super::regs;
use super::trb::{
    CompletionCode, TRB_CYCLE, TRB_TYPE_COMMAND_COMPLETION, TRB_TYPE_LINK, TRB_TYPE_MASK, Trb,
};
use crate::mm::dma::{DmaBuffer, MmioMapping};
use crate::pcie::enumrate::{Bdf, MemoryBar, PCIConfigSpace, decode_memory_bar, doit};

const POLL_LIMIT: usize = 100_000;
const MIN_BAR_LEN: u64 = 0x1000;
const EVENT_RING_ENTRIES: usize = 32;
const COMMAND_RING_ENTRIES: usize = 4096 / core::mem::size_of::<Trb>();

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
    runtime: usize,
    doorbell: usize,
    op: usize,
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
                    control: TRB_TYPE_LINK | TRB_CYCLE,
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
            runtime: 0,
            doorbell: 0,
            op: 0,
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
            register.write32(runtime + regs::RT_INTR0 + regs::IMAN, regs::IMAN_IP); // polling: IE stays clear
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
        let command = Trb {
            control: regs::TRB_TYPE_NOOP_COMMAND | TRB_CYCLE,
            ..Trb::default()
        };
        unsafe { core::ptr::write_volatile(self.command_ring.as_ptr() as *mut Trb, command) };
        unsafe { core::ptr::write_volatile(base.add(doorbell) as *mut u32, regs::DB_TARGET) };
        let completion = self.poll_command(register, self.command_ring.physical_address());
        match completion {
            Ok(()) => {
                crate::println!(
                    "[INFO] xhci: running, MaxSlots {}, context {}, scratchpads {}, command completion",
                    max_slots,
                    if context_64 { 64 } else { 32 },
                    self._scratchpads.len()
                );
                Ok(self)
            }
            Err(error) => Err(error),
        }
    }

    fn poll_command(
        &mut self,
        register: regs::RegisterBlock,
        command_address: u64,
    ) -> Result<(), InitError> {
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
                    self.event_ring.physical_address() + (self.event_consumer * 16) as u64,
                );
            }
            if let Some(code) = command_completion(trb, command_address) {
                return if code == CompletionCode::Success {
                    Ok(())
                } else {
                    Err(InitError::Command(code))
                };
            }
        }
        Err(InitError::Timeout("command completion"))
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
