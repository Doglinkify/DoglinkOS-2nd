use core::{
    fmt::Write,
    sync::atomic::{AtomicBool, Ordering},
};

use x86_64::instructions::port::{PortReadOnly, PortWriteOnly};

use crate::console::ECHO_FLAG;

const PORT: u16 = 0x3f8;
pub(super) static SERIAL_OK: AtomicBool = AtomicBool::new(false);

#[inline]
unsafe fn outb(port: u16, value: u8) {
    PortWriteOnly::new(port).write(value);
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    PortReadOnly::new(port).read()
}

pub(super) fn init() {
    unsafe {
        outb(PORT + 1, 0x00); // Disable all interrupts
        outb(PORT + 3, 0x80); // Enable DLAB (set baud rate divisor)
        outb(PORT, 0x01); // Set divisor to 1 (lo byte) 115200 baud
        outb(PORT + 1, 0x00); //                  (hi byte)
        outb(PORT + 3, 0x03); // 8 bits, no parity, one stop bit
        outb(PORT + 2, 0xC7); // Enable FIFO, clear them, with 14-byte threshold
        outb(PORT + 4, 0x0B); // IRQs enabled, RTS/DSR set
        outb(PORT + 4, 0x1E); // Set in loopback mode, test the serial chip
        outb(PORT, 0xAE); // Test serial chip (send byte 0xAE and check if serial returns same byte)
        if inb(PORT) != 0xAE {
            return;
        }
        outb(PORT + 4, 0x0F);
        SERIAL_OK.store(true, Ordering::Relaxed);
    }
}

fn received() -> bool {
    unsafe { inb(PORT + 5) & 1 == 1 }
}

/// Non-blocking read
pub fn read() -> Option<u8> {
    if received() {
        let mut res = unsafe { inb(PORT) };
        if res == 0x0d {
            // translate CR to LF
            res = 0x0a;
        }
        if ECHO_FLAG.load(Ordering::Relaxed) {
            super::TERMINAL.lock().process(&[res]);
            write(res);
        }
        Some(res)
    } else {
        None
    }
}

fn is_transmit_empty() -> bool {
    unsafe { inb(PORT + 5) & 0x20 == 0x20 }
}

/// Blocking write
pub fn write(data: u8) {
    while !is_transmit_empty() {}
    unsafe { outb(PORT, data) }
}

/// Blocking write bytes
pub fn write_bytes(buf: &[u8]) {
    for &c in buf {
        write(c);
    }
}

pub struct Serial;

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        write_bytes(s.as_bytes());
        Ok(())
    }
}
