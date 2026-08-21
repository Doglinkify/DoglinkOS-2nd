use alloc::{boxed::Box, format, string::String, vec::Vec};
use spin::{lazy::Lazy, mutex::Mutex};

mod rtl8139;

trait Nic: Send {
    fn mac(&self) -> [u8; 6];

    #[allow(dead_code)]
    fn poll(&mut self);

    fn format_mac(&self) -> String {
        let mac = self.mac();
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        )
    }
}

static NICS: Lazy<Mutex<Vec<Box<dyn Nic>>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub fn init() {
    rtl8139::init();
}

pub fn poll() {
    let mut nics = NICS.lock();
    for nic in nics.iter_mut() {
        nic.poll();
    }
}
