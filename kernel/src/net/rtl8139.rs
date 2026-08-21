use alloc::boxed::Box;
use x86_64::instructions::port::{PortReadOnly, PortWriteOnly};

use crate::{
    mm::{dma::DmaBuffer, page_alloc::PAGE_SIZE},
    net::Nic,
    pcie::enumrate::PCIConfigSpace,
    println,
};

struct Rtl8139 {
    io_base: u16,
    mac: [u8; 6],
    rx_buffer: DmaBuffer,
    cur_rx: u16,
}

impl Rtl8139 {
    pub fn new(config: &PCIConfigSpace) -> Self {
        // enable Bus Mastering
        let command = config.read_u16(4);
        unsafe { config.write_u16(4, command | (1 << 2)) }

        // BAR0 is I/O Space BAR
        let io_base = (config.bar[0] & !0b11) as u16;
        Self::fromio_base(io_base)
    }

    fn fromio_base(io_base: u16) -> Self {
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
            PortWriteOnly::new(io_base + 0x44).write(0xfu8);
        }

        // enable receive and transmitter
        unsafe {
            PortWriteOnly::new(io_base + 0x37).write(0x0cu8);
        }

        Self {
            io_base,
            mac,
            rx_buffer,
            cur_rx: 0,
        }
    }
}

unsafe impl Send for Rtl8139 {}

impl super::Nic for Rtl8139 {
    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn poll(&mut self) {
        const ROK: u16 = 0x0001;
        const TOK: u16 = 0x0004;
        let status: u16 = unsafe { PortReadOnly::new(self.io_base + 0x3e).read() };
        unsafe {
            PortWriteOnly::new(self.io_base + 0x3e).write(status);
        }
        if status & TOK != 0 {
            println!("[DEBUG] rtl8139: sent packet");
        }
        if status & ROK != 0 {
            let mut read_index = self.cur_rx;
            let rx_buffer_ptr = self.rx_buffer.as_ptr();
            let rx_header: u32 = unsafe { *(rx_buffer_ptr.add(read_index as usize) as *const _) };
            let rx_status = (rx_header & 0xffff) as u16;
            let rx_size = (rx_header >> 16) as u16;
            if rx_status & 0x0001 != 0 {
                let packet = unsafe {
                    let data_ptr = rx_buffer_ptr.add(read_index as usize + 4).cast_const();
                    let len = rx_size as usize - 4;
                    core::slice::from_raw_parts(data_ptr, len)
                };
                println!("[DEBUG] rtl8139: rx packet {packet:?}");
            }
            read_index += rx_size + 7;
            read_index &= !3;
            if read_index > 8192 {
                read_index -= 8192;
            }
            unsafe {
                PortWriteOnly::new(self.io_base + 0x38).write(read_index - 16);
            }
            self.cur_rx = read_index;
        }
    }
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
