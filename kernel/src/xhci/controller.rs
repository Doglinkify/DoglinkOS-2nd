//! PCI discovery and the bounded xHCI controller reset sequence.

use super::regs;
use crate::mm::dma::MmioMapping;
use crate::pcie::enumrate::{Bdf, MemoryBar, PCIConfigSpace, decode_memory_bar, doit};

const POLL_LIMIT: usize = 100_000;
const MIN_BAR_LEN: u64 = 0x1000;

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

fn reset_controller(base: *mut u8) -> Result<(u16, u8), ResetError> {
    let regs = unsafe { regs::RegisterBlock::new(base) };
    let cap_len = unsafe { regs.read32(regs::CAPLENGTH) } as usize & 0xff;
    let version = unsafe { regs.read16(regs::HCIVERSION) };
    let hcs1 = unsafe { regs.read32(regs::HCSPARAMS1) };
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
    Ok((version, port_count))
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
        Ok((version, ports)) => {
            crate::println!(
                "[INFO] xhci: {:02x}:{:02x}.{} version {:x}, ports {}, reset successful",
                bdf.bus,
                bdf.device,
                bdf.function,
                version,
                ports
            );
            ControllerState::Reset
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
}
