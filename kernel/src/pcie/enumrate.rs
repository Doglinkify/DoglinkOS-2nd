use crate::mm::phys_to_virt;
use spin::Mutex;

pub static pcie_mmio_base: Mutex<u64> = Mutex::new(0);

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
    pub BIST: u8,
    pub bar: [u32; 6],
    // TODO
}

pub fn get_config_space(bus: u8, device: u8, function: u8) -> &'static PCIConfigSpace {
    unsafe {
        &*((*pcie_mmio_base.lock()
            + ((bus as u64) << 20)
            + ((device as u64) << 15)
            + ((function as u64) << 12)) as *const PCIConfigSpace)
    }
}

pub fn check<F>(bus: u8, device: u8, function: u8, mut hook: F) -> bool
where
    F: FnMut(u8, u8, u8, &PCIConfigSpace),
{
    let config = get_config_space(bus, device, function);
    if config.vendor_id != 65535 && config.vendor_id != 0 {
        hook(bus, device, function, config);
    }
    config.vendor_id != 65535 && config.vendor_id != 0 && config.header_type & 0x80 == 0x80
}

pub fn init() {
    crate::println!("[INFO] pcie: init() called");
    *pcie_mmio_base.lock() = phys_to_virt(crate::acpi::parse_mcfg());
}

pub fn doit<F>(mut hook: F)
where
    F: FnMut(u8, u8, u8, &PCIConfigSpace),
{
    for bus in 0..=255 {
        for device in 0..32 {
            if check(bus, device, 0, &mut hook) {
                for function in 1..8 {
                    check(bus, device, function, &mut hook);
                }
            }
        }
    }
}
