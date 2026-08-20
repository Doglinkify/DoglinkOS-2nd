use x86_64::instructions::port::PortReadOnly;

use crate::println;

pub(super) fn init() {
    crate::pcie::enumrate::doit(|bus, device, function, config| {
        if config.vendor_id == 0x10ec && config.device_id == 0x8139 {
            println!("[INFO] rtl8139: found at {bus:02x}:{device:02x}.{function}");
            let command = config.read_u16(4);
            unsafe { config.write_u16(4, command | (1 << 2)) }
            let io_base = (config.bar[0] & !0b11) as u16;
            let mut mac = [0u8; 6];
            for i in 0..6 {
                mac[i] = unsafe { PortReadOnly::new(io_base + i as u16).read() };
            }
            println!(
                "[INFO] rtl8139: physical address is {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
            );
        }
    });
}
