use crate::acpi::mcfg;
use crate::mm::phys_to_virt;
use crate::{print, println};
use spin::Mutex;

pub static pcie_mmio_base: Mutex<u64> = Mutex::new(0);

#[derive(Debug)]
#[repr(C)]
pub struct PCIConfigSpace {
    vendor_id: u16,
    device_id: u16,
    command: u16,
    status: u16,
    revision_id: u8,
    prog_if: u8,
    subclass: u8,
    class_code: u8,
    cache_line_size: u8,
    latency_timer: u8,
    header_type: u8,
    BIST: u8,
    bar: [u32; 6],
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

pub fn check(bus: u8, device: u8, function: u8) -> bool {
    let config = get_config_space(bus, device, function);
    if config.vendor_id != 65535 {
        let vendor_id = config.vendor_id;
        let device_id = config.device_id;
        println!(
            "{:02x}:{:02x}.{}: {:02x}{:02x}: {:04x}:{:04x}",
            bus,
            device,
            function,
            config.class_code,
            config.subclass,
            vendor_id,
            device_id
        );
        print!("PROG_IF={} ", config.prog_if);
        for i in 0..6 {
            print!(" BAR{}={:x}", i, config.bar[i]);
        }
        println!();
    }
    config.vendor_id != 65535 && config.header_type & 0x80 == 0x80
}

pub fn doit() {
    println!("PCI enumerating result:");
    *pcie_mmio_base.lock() = phys_to_virt((*mcfg.lock()).alloc.base_addr);
    for bus in 0..=255 {
        for device in 0..32 {
            if check(bus, device, 0) {
                for function in 1..8 {
                    check(bus, device, function);
                }
            }
        }
    }
}
