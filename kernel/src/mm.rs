use crate::println;
use limine::response::HhdmResponse;
use spin::Mutex;
use good_memory_allocator::SpinLockedAllocator;

#[global_allocator]
static ALLOCATOR: SpinLockedAllocator = SpinLockedAllocator::empty();

pub static offset: Mutex<u64> = Mutex::new(0);

pub fn init(res: &HhdmResponse) {
    println!("[INFO] mm: init() called");
    *offset.lock() = res.offset();
    let heap_address = phys_to_virt(0x10000);
    println!("[INFO] RESERVED_HEAP is at {:?}", heap_address as *const ());
    unsafe {
        ALLOCATOR.init(heap_address as usize, 8 * 1024 * 1024);
    }
}

pub fn phys_to_virt(addr: u64) -> u64 {
    addr + *offset.lock()
}
