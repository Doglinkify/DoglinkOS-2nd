use limine::response::HhdmResponse;
use crate::println;

pub static mut offset: u64 = 0;

pub fn init(res: &HhdmResponse) {
    unsafe {
        offset = res.offset();
        // println!("{offset}");
    }
}

pub fn phys_to_virt(addr: u64) -> u64 {
    unsafe {
        addr + offset
    }
}
