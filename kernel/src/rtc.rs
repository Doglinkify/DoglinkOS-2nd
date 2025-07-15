pub fn prepare_sleep() {
    poll_uip();
}

pub fn perform_sleep() {
    poll_uip();
}

fn poll_uip() {
    let mut select = x86_64::instructions::port::PortWriteOnly::<u8>::new(0x70);
    let mut data = x86_64::instructions::port::PortReadOnly::<u8>::new(0x71);
    unsafe {
        select.write(0x8a);
        while data.read() & 0x80 != 0x80 {
            select.write(0x8a);
        }
        while data.read() & 0x80 != 0 {
            select.write(0x8a);
        }
    }
}
