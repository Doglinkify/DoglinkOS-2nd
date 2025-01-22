use crate::println;
use limine::response::HhdmResponse;
use spin::Mutex;
use good_memory_allocator::SpinLockedAllocator;

#[global_allocator]
static ALLOCATOR: SpinLockedAllocator = SpinLockedAllocator::empty();

#[used]
static mut RESERVED_HEAP: [u8; 8 * 1024 * 1024] = [0; 8 * 1024 * 1024];

pub static offset: Mutex<u64> = Mutex::new(0);

pub fn init(res: &HhdmResponse) {
    println!("[INFO] mm: init() called");
    unsafe {
        println!("[INFO] RESERVED_HEAP is at {:?}", &RESERVED_HEAP as *const u8);
    }
    *offset.lock() = res.offset();
    unsafe {
        ALLOCATOR.init(&mut RESERVED_HEAP as *mut u8 as usize, 8 * 1024 * 1024);
    }
}

pub fn phys_to_virt(addr: u64) -> u64 {
    addr + *offset.lock()
}
