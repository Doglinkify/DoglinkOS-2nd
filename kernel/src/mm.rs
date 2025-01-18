use crate::println;
use limine::response::HhdmResponse;
use spin::Mutex;

pub static offset: Mutex<u64> = Mutex::new(0);

pub fn init(res: &HhdmResponse) {
    *offset.lock() = res.offset();
}

pub fn phys_to_virt(addr: u64) -> u64 {
    addr + *offset.lock()
}
