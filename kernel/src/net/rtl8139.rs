use alloc::boxed::Box;
use x86_64::instructions::port::{PortReadOnly, PortWriteOnly};

use crate::{
    mm::{dma::DmaBuffer, page_alloc::PAGE_SIZE},
    net::Nic,
    pcie::enumrate::PCIConfigSpace,
    println,
};

struct Rtl8139 {
    _io_base: u16,
    mac: [u8; 6],
    _rx_buffer: DmaBuffer,
}

impl Rtl8139 {
    pub fn new(config: &PCIConfigSpace) -> Self {
        // enable Bus Mastering
        let command = config.read_u16(4);
        unsafe { config.write_u16(4, command | (1 << 2)) }

        // BAR0 is I/O Space BAR
        let io_base = (config.bar[0] & !0b11) as u16;
        Self::from_io_base(io_base)
    }

    fn from_io_base(io_base: u16) -> Self {
        // read MAC address
        let mut mac = [0u8; 6];
        for i in 0..6 {
            mac[i] = unsafe { PortReadOnly::new(io_base + i as u16).read() };
        }

        // software reset
        unsafe {
            PortWriteOnly::new(io_base + 0x37).write(0x10u8);
        }
        while unsafe { PortReadOnly::<u8>::new(io_base + 0x37).read() } & 0x10 != 0 {}

        // init receive buffer
        let rx_buffer = DmaBuffer::new(8208, PAGE_SIZE).unwrap();
        let phys_addr: u32 = rx_buffer
            .physical_address()
            .try_into()
            .expect("DMA buffer address cannot fit in 32-bit");
        unsafe {
            PortWriteOnly::new(io_base + 0x30).write(phys_addr);
        }

        // skip setting up IMR+ISR because we use polling for now

        // configure receive buffer
        unsafe {
            PortWriteOnly::new(io_base + 0x44).write(0xfu8 | (1 << 7));
        }

        // do not enable receive and transmitter now because we don't have polling logic yet

        Self {
            _io_base: io_base,
            mac,
            _rx_buffer: rx_buffer,
        }
    }
}

unsafe impl Send for Rtl8139 {}

impl super::Nic for Rtl8139 {
    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn poll(&self) {}
}

pub(super) fn init() {
    crate::pcie::enumrate::doit(|bus, device, function, config| {
        if config.vendor_id == 0x10ec && config.device_id == 0x8139 {
            println!("[INFO] rtl8139: found at {bus:02x}:{device:02x}.{function}");
            let nic = Rtl8139::new(config);
            println!("[INFO] rtl8139: physical address is {}", nic.format_mac());
            super::NICS.lock().push(Box::new(nic));
        }
    });
}
