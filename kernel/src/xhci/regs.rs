//! Volatile register layout helpers. Offsets are supplied by CAPLENGTH,
//! DBOFF and RTSOFF; no QEMU-specific offsets are assumed.

use core::ptr::{read_volatile, write_volatile};

#[derive(Clone, Copy)]
pub struct RegisterBlock {
    base: *mut u8,
}

impl RegisterBlock {
    /// # Safety
    /// `base` must point at the controller's mapped MMIO BAR.
    pub const unsafe fn new(base: *mut u8) -> Self {
        Self { base }
    }

    pub const fn base(self) -> *mut u8 {
        self.base
    }

    pub unsafe fn read16(self, offset: usize) -> u16 {
        unsafe { read_volatile(self.base.add(offset).cast()) }
    }

    pub unsafe fn read32(self, offset: usize) -> u32 {
        unsafe { read_volatile(self.base.add(offset).cast()) }
    }

    pub unsafe fn write32(self, offset: usize, value: u32) {
        unsafe { write_volatile(self.base.add(offset).cast(), value) }
    }

    pub unsafe fn read64(self, offset: usize) -> u64 {
        unsafe { read_volatile(self.base.add(offset).cast()) }
    }

    pub unsafe fn write64(self, offset: usize, value: u64) {
        unsafe { write_volatile(self.base.add(offset).cast(), value) }
    }
}

pub const CAPLENGTH: usize = 0x00;
pub const HCIVERSION: usize = 0x02;
pub const HCSPARAMS1: usize = 0x04;
pub const HCCPARAMS1: usize = 0x10;
pub const DBOFF: usize = 0x14;
pub const RTSOFF: usize = 0x18;
pub const USBCMD: usize = 0x00;
pub const USBSTS: usize = 0x04;
pub const CONFIG: usize = 0x38;
pub const DCBAAP: usize = 0x30;
pub const CRCR: usize = 0x18;

pub const USBCMD_RUN: u32 = 1;
pub const USBCMD_HCRST: u32 = 1 << 1;
pub const USBSTS_HCHALTED: u32 = 1;
pub const USBSTS_CNR: u32 = 1 << 11;

pub const HCCPARAMS1_XECP_MASK: u32 = 0xff00_0000;
pub const HCCPARAMS1_XECP_SHIFT: u32 = 16;

pub const XECAP_ID_MASK: u32 = 0xff;
pub const XECAP_NEXT_MASK: u32 = 0xffff << 8;
pub const XECAP_NEXT_SHIFT: u32 = 8;
pub const XECAP_USB_LEGACY_SUPPORT: u32 = 1;
pub const USBLEGSUP_BIOS_OWNED: u32 = 1 << 16;
pub const USBLEGSUP_OS_OWNED: u32 = 1 << 24;
