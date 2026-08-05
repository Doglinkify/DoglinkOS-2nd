use x86_64::instructions::port::{PortReadOnly, PortWriteOnly};

pub(crate) fn play_sound(freq: u32) {
    let div = 1193180 / freq;
    unsafe {
        PortWriteOnly::new(0x43).write(0xb6u8);
        PortWriteOnly::new(0x42).write((div & 0xff) as u8);
        PortWriteOnly::new(0x42).write(((div >> 8) & 0xff) as u8);
    }
    let tmp: u8 = unsafe { PortReadOnly::new(0x61).read() };
    if tmp != (tmp | 3) {
        unsafe { PortWriteOnly::new(0x61).write(tmp | 3) };
    }
}

pub(crate) fn stop_sound() {
    let tmp: u8 = unsafe { PortReadOnly::new(0x61).read() };
    unsafe { PortWriteOnly::new(0x61).write(tmp & 0xfc) };
}
