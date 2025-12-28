use crate::{acpi::PCI_CONFIG_REGIONS, mm::phys_to_virt};

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

pub fn doit<F>(mut hook: F)
where
    F: FnMut(u8, u8, u8, &PCIConfigSpace),
{
    for region in PCI_CONFIG_REGIONS.iter() {
        // crate::println!("[DEBUG] pcie: found config region, segment_group = {}, bus_range = {:?}, physical_address = 0x{:x}", region.segment_group, region.bus_range, region.physical_address);
        for bus in region.bus_range.clone() {
            for device in 0..32 {
                if check(
                    phys_to_virt(region.physical_address as u64),
                    bus,
                    device,
                    0,
                    &mut hook,
                ) {
                    for function in 1..8 {
                        check(
                            phys_to_virt(region.physical_address as u64),
                            bus,
                            device,
                            function,
                            &mut hook,
                        );
                    }
                }
            }
        }
    }
}
