use crate::{acpi::PCI_CONFIG_REGIONS, mm::phys_to_virt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bdf {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl Bdf {
    pub const fn new(bus: u8, device: u8, function: u8) -> Option<Self> {
        if device < 32 && function < 8 {
            Some(Self {
                bus,
                device,
                function,
            })
        } else {
            None
        }
    }

    pub const fn offset(self, bus_start: u8) -> Option<u64> {
        if self.bus < bus_start {
            return None;
        }
        Some(
            ((self.bus - bus_start) as u64) << 20
                | (self.device as u64) << 15
                | (self.function as u64) << 12,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryBar {
    Bits32 { address: u64 },
    Bits64 { address: u64 },
}

pub fn decode_memory_bar(low: u32, high: u32) -> Option<MemoryBar> {
    if low & 1 != 0 {
        return None;
    }
    let kind = (low >> 1) & 3;
    let address = (low as u64 & 0xfffffff0) | ((high as u64) << 32);
    match kind {
        0 => Some(MemoryBar::Bits32 { address }),
        2 => Some(MemoryBar::Bits64 { address }),
        _ => None,
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct PCIConfigSpace {
    pub vendor_id: u16,
    pub device_id: u16,
    pub command: u16,
    pub status: u16,
    pub revision_id: u8,
    pub prog_if: u8,
    pub subclass: u8,
    pub class_code: u8,
    pub cache_line_size: u8,
    pub latency_timer: u8,
    pub header_type: u8,
    pub bist: u8,
    pub bar: [u32; 6],
    // TODO
}

impl PCIConfigSpace {
    pub fn read_u8(&self, offset: usize) -> u8 {
        assert!(offset < 4096);
        unsafe { core::ptr::read_volatile((self as *const _ as *const u8).add(offset)) }
    }

    pub fn read_u16(&self, offset: usize) -> u16 {
        assert!(offset + 2 <= 4096 && offset & 1 == 0);
        unsafe { core::ptr::read_volatile((self as *const _ as *const u16).add(offset / 2)) }
    }

    pub fn read_u32(&self, offset: usize) -> u32 {
        assert!(offset + 4 <= 4096 && offset & 3 == 0);
        unsafe { core::ptr::read_volatile((self as *const _ as *const u32).add(offset / 4)) }
    }

    pub unsafe fn write_u8(&self, offset: usize, value: u8) {
        assert!(offset + 1 <= 4096);
        unsafe { core::ptr::write_volatile((self as *const _ as *mut u8).add(offset), value) }
    }

    pub unsafe fn write_u16(&self, offset: usize, value: u16) {
        assert!(offset + 2 <= 4096 && offset & 1 == 0);
        unsafe { core::ptr::write_volatile((self as *const _ as *mut u16).add(offset / 2), value) }
    }

    pub unsafe fn write_u32(&self, offset: usize, value: u32) {
        assert!(offset + 4 <= 4096 && offset & 3 == 0);
        unsafe { core::ptr::write_volatile((self as *const _ as *mut u32).add(offset / 4), value) }
    }

    pub fn command(&self) -> u16 {
        self.read_u16(0x04)
    }

    pub unsafe fn update_command(&self, set: u16, clear: u16) {
        let value = (self.command() | set) & !clear;
        unsafe {
            self.write_u16(0x04, value);
        }
    }

    pub fn capabilities(&self) -> CapabilityIter<'_> {
        CapabilityIter {
            config: self,
            next: if self.read_u16(0x06) & (1 << 4) != 0 {
                self.read_u8(0x34)
            } else {
                0
            },
            seen: [0; 256],
            count: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capability {
    pub id: u8,
    pub offset: u8,
}

pub struct CapabilityIter<'a> {
    config: &'a PCIConfigSpace,
    next: u8,
    seen: [u8; 256],
    count: usize,
}

impl Iterator for CapabilityIter<'_> {
    type Item = Capability;

    fn next(&mut self) -> Option<Self::Item> {
        let offset = self.next;
        if offset < 0x40
            || offset & 3 != 0
            || self.count >= self.seen.len()
            || self.seen[offset as usize] != 0
        {
            self.next = 0;
            return None;
        }
        self.seen[offset as usize] = 1;
        self.count += 1;
        let id = self.config.read_u8(offset as usize);
        let next = self.config.read_u8(offset as usize + 1);
        self.next = if next == 0 { 0 } else { next };
        Some(Capability { id, offset })
    }
}

pub fn get_config_space(
    mmio_base: u64,
    bus: u8,
    device: u8,
    function: u8,
) -> &'static PCIConfigSpace {
    unsafe {
        &*((mmio_base + ((bus as u64) << 20) + ((device as u64) << 15) + ((function as u64) << 12))
            as *const PCIConfigSpace)
    }
}

pub fn check<F>(mmio_base: u64, bus: u8, device: u8, function: u8, mut hook: F) -> bool
where
    F: FnMut(u8, u8, u8, &PCIConfigSpace),
{
    let config = get_config_space(mmio_base, bus, device, function);
    if config.vendor_id != 65535 && config.vendor_id != 0 {
        hook(bus, device, function, config);
    }
    config.vendor_id != 65535 && config.vendor_id != 0 && config.header_type & 0x80 == 0x80
}

fn check_at<F>(base: u64, bdf: Bdf, mut hook: F) -> bool
where
    F: FnMut(u8, u8, u8, &PCIConfigSpace),
{
    let config = get_config_space(base, 0, bdf.device, bdf.function);
    let present = config.vendor_id != 0xffff && config.vendor_id != 0;
    if present {
        hook(bdf.bus, bdf.device, bdf.function, config);
    }
    present && config.header_type & 0x80 != 0
}

pub fn doit<F>(mut hook: F)
where
    F: FnMut(u8, u8, u8, &PCIConfigSpace),
{
    for region in PCI_CONFIG_REGIONS.iter() {
        // crate::println!("[DEBUG] pcie: found config region, segment_group = {}, bus_range = {:?}, physical_address = 0x{:x}", region.segment_group, region.bus_range, region.physical_address);
        for bus in region.bus_range.clone() {
            for device in 0..32 {
                let base = phys_to_virt(region.physical_address as u64)
                    + (((bus - *region.bus_range.start()) as u64) << 20);
                let bdf = Bdf::new(bus, device, 0).unwrap();
                if check_at(base, bdf, &mut hook) {
                    for function in 1..8 {
                        check_at(base, Bdf::new(bus, device, function).unwrap(), &mut hook);
                    }
                }
            }
        }
    }
}

pub fn test() {
    let bdf = Bdf::new(0x23, 4, 2).unwrap();
    assert_eq!(bdf.offset(0x20), Some(0x300000 + 4 * 0x8000 + 2 * 0x1000));
    assert_eq!(bdf.offset(0x24), None);
    assert!(Bdf::new(32, 0, 0).is_some());
    assert!(Bdf::new(0, 32, 0).is_none());
    assert_eq!(
        decode_memory_bar(0x12345000, 0),
        Some(MemoryBar::Bits32 {
            address: 0x12345000
        })
    );
    assert_eq!(
        decode_memory_bar(0x12345004, 0xdeadbeef),
        Some(MemoryBar::Bits64 {
            address: 0xdeadbeef12345000
        })
    );
    assert_eq!(decode_memory_bar(1, 0), None);
    assert_eq!(decode_memory_bar(0x6, 0), None);
    #[repr(C, align(4))]
    struct ConfigBytes([u8; 4096]);
    let mut bytes = ConfigBytes([0u8; 4096]);
    bytes.0[0x06] = 0x10;
    bytes.0[0x34] = 0x40;
    bytes.0[0x40] = 1;
    bytes.0[0x41] = 0x48;
    bytes.0[0x48] = 5;
    bytes.0[0x49] = 0x40;
    let config = unsafe { &*(&bytes.0 as *const _ as *const PCIConfigSpace) };
    let mut caps = config.capabilities();
    assert_eq!(
        caps.next(),
        Some(Capability {
            id: 1,
            offset: 0x40
        })
    );
    assert_eq!(
        caps.next(),
        Some(Capability {
            id: 5,
            offset: 0x48
        })
    );
    assert_eq!(caps.next(), None);
    crate::println!("[INFO] pcie: typed access self-test passed");
}
