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

fn read_data() -> u8 {
    wait_read();
    unsafe { PortReadOnly::new(0x60).read() }
}

fn write_command(cmd: u8) {
    wait_write();
    unsafe { PortWriteOnly::new(0x64).write(cmd) }
}

fn write_data(cmd: u8) {
    wait_write();
    unsafe { PortWriteOnly::new(0x60).write(cmd) }
}

fn flush_output() {
    let mut port: PortReadOnly<u8> = PortReadOnly::new(0x64);
    for _ in 0..64 {
        unsafe {
            if port.read() & 0x01 == 0 {
                break;
            }
            _ = read_data();
        }
    }
}

fn read_config() -> u8 {
    write_command(0x20); // read_config
    read_data()
}

fn write_config(config: u8) {
    write_command(0x60); // write_config
    write_data(config)
}

fn send_to_port1(cmd: u8) {
    for _ in 0..3 {
        write_data(cmd);
        let response = read_data();
        if response == 0xfa {
            break;
        }
    }
}

fn send_to_port2(cmd: u8) {
    for _ in 0..3 {
        write_command(0xd4);
        write_data(cmd);
        let response = read_data();
        if response == 0xfa {
            break;
        }
    }
}

pub fn handle_mouse(packet: u8) {
    static CURRENT_PACKET: AtomicU8 = AtomicU8::new(0);
    static FLAGS: AtomicI16 = AtomicI16::new(0);
    static X: AtomicI16 = AtomicI16::new(0);
    // println!(
    //     "debug: handle_mouse() called, CURRENT_PACKET = {}",
    //     CURRENT_PACKET.load(Ordering::Relaxed)
    // );
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
    init_controller();
    init_keyboard();
    init_mouse();
}

fn init_controller() {
    write_command(0xad); // disable_port1
    write_command(0xa7); // disable_port2
    flush_output();
    let config = read_config() & 0b10111100u8;
    write_config(config);
    write_command(0xaa); // test_controller
    let response = read_data();
    assert!(response == 0x55);
    write_config(config);
    write_command(0xa8); // enable_port2
    let config2 = read_config();
    assert!(config2 & 0x20 == 0);
    write_command(0xa7); // disable_port2
    write_command(0xab); // test_port1
    let response2 = read_data();
    assert!(response2 == 0);
    write_command(0xa9); // test_port2
    let response3 = read_data();
    assert!(response3 == 0);
}

fn init_keyboard() {
    write_command(0xae); // enable_port1
    flush_output();
    send_to_port1(0xff); // dev_reset
    let response = read_data();
    assert!(response == 0xaa);
    send_to_port1(0xf0);
    send_to_port1(0x01);
    send_to_port1(0xf4); // dev_enable
    let mut config = read_config();
    config |= 1;
    config &= 0b10101111;
    write_config(config);
}

fn init_mouse() {
    write_command(0xa8); // enable_port2
    flush_output();
    send_to_port2(0xff); // dev_reset
    let response = read_data();
    assert!(response == 0xaa);
    let data = read_data();
    assert!(data == 0);
    send_to_port2(0xf4); // dev_enable
    send_to_port2(0xf3); // set_sample_rate
    send_to_port2(10);
    send_to_port2(0xf2);
    let mut config = read_config();
    config |= 0b00000011;
    config &= 0b10001111;
    write_config(config);
}

pub fn interrupt_handler() {
    unsafe {
        let status: u8 = PortReadOnly::new(0x64).read();
        if status & 1 == 0 {
            return;
        }
        let data: u8 = PortReadOnly::new(0x60).read();
        if status & 0x20 == 0x20 {
            handle_mouse(data);
        } else {
            let scancode = data;
            let mut term = crate::console::TERMINAL.lock();
            term.handle_keyboard(scancode);
            let echo = crate::console::ECHO_FLAG.load(core::sync::atomic::Ordering::Relaxed);
            while let Some(b) = crate::console::ECHO_BUFFER.pop() {
                if echo {
                    term.process(&[b]);
                }
                crate::console::INPUT_BUFFER.force_push(b);
            }
        }
    }
}
