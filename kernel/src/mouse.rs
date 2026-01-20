use core::sync::atomic::{AtomicI16, AtomicU8, Ordering};

use os_terminal::MouseInput;
use x86_64::instructions::port::{PortReadOnly, PortWriteOnly};

fn wait_write() {
    let mut port: PortReadOnly<u8> = PortReadOnly::new(0x64);
    unsafe { while port.read() & 0x02 != 0 {} }
}

fn wait_read() {
    let mut port: PortReadOnly<u8> = PortReadOnly::new(0x64);
    unsafe { while port.read() & 0x01 == 0 {} }
}

fn port_send(cmd: u8) {
    unsafe {
        wait_write();
        PortWriteOnly::new(0x64).write(0xd4u8);
        wait_write();
        PortWriteOnly::new(0x60).write(cmd);
        wait_read();
        assert!(PortReadOnly::<u8>::new(0x60).read() == 0xfa);
    }
}

pub fn port_read() -> u8 {
    wait_read();
    unsafe { PortReadOnly::new(0x60).read() }
}

pub fn handle(packet: u8) {
    static CURRENT_PACKET: AtomicU8 = AtomicU8::new(0);
    static FLAGS: AtomicI16 = AtomicI16::new(0);
    static X: AtomicI16 = AtomicI16::new(0);
    match CURRENT_PACKET.load(Ordering::Relaxed) {
        0 => {
            if (packet >> 3) & 1 == 1 {
                // println!("[DEBUG] mouse: raw flags = 0b{:08b}", packet);
                FLAGS.store(packet as i16, Ordering::Relaxed);
                CURRENT_PACKET.store(1, Ordering::Relaxed);
            }
        }
        1 => {
            X.store(
                packet as i16 - ((FLAGS.load(Ordering::Relaxed) << 4) & 0x100),
                Ordering::Relaxed,
            );
            CURRENT_PACKET.store(2, Ordering::Relaxed);
        }
        2 => {
            let flags = FLAGS.load(Ordering::Relaxed);
            let x = X.load(Ordering::Relaxed);
            _ = x;
            let y = packet as i16 - ((flags << 3) & 0x100);
            // crate::println!(
            //     "[DEBUG] mouse report x: {}, y: {}, middle button: {}, right button: {}, left button: {}",
            //     x, y,
            //     (flags >> 2) & 1,
            //     (flags >> 1) & 1,
            //     flags & 1
            // );
            if (flags >> 1) & 1 == 1 {
                let mut terminal = crate::console::TERMINAL.lock();
                terminal.handle_mouse(MouseInput::Scroll(y as isize));
                let echo = crate::console::ECHO_FLAG.load(core::sync::atomic::Ordering::Relaxed);
                while let Some(b) = crate::console::ECHO_BUFFER.pop() {
                    if echo {
                        terminal.process(&[b]);
                    }
                    crate::console::INPUT_BUFFER.force_push(b);
                }
            }
            CURRENT_PACKET.store(0, Ordering::Relaxed);
        }
        _ => unreachable!(),
    }
}

pub fn init() {
    port_send(0xf4);
    port_send(0xf3);
    port_send(10);
    port_send(0xf2);
}
