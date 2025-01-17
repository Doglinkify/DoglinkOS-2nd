use crate::acpi::mcfg;
use crate::mm::phys_to_virt;
use crate::println;

pub static mut pcie_mmio_base: u64 = 0;
pub static mut bus_range: core::ops::RangeInclusive<u8> = 0..=0;
#[derive(Debug)]
#[repr(packed)]
pub struct PCIConfigSpace {
    vendor_id: u16,
    device_id: u16,
    command: u16,
    status: u16,
    prog_if: u16,
    subclass: u8,
    class_code: u8,
    // TODO
}

pub fn get_config_space(bus: u8, device: u8) -> &'static PCIConfigSpace {
    unsafe {
        &*((pcie_mmio_base + ((bus as u64) << 20) + ((device as u64) << 15)) as * const PCIConfigSpace)
    }
}

pub fn doit() {
    unsafe {
        pcie_mmio_base = phys_to_virt((*mcfg).alloc.base_addr);
        bus_range = ((*mcfg).alloc.start_pci_bus_number)..=((*mcfg).alloc.end_pci_bus_number);
        for bus in bus_range.clone() {
            for device in 0..32 {
                let config = get_config_space(bus, device);
                if config.vendor_id != 65535 {
                    println!("PCI bus {} device {}: {:?}", bus, device, *config);
                }
            }
        }
    }
}
