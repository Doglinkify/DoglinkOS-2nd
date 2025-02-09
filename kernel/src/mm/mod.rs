pub mod bitmap;
pub mod page_alloc;

use limine::request::HhdmRequest;
use spin::Mutex;
use good_memory_allocator::SpinLockedAllocator;

#[used]
#[link_section = ".requests"]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[global_allocator]
static ALLOCATOR: SpinLockedAllocator = SpinLockedAllocator::empty();

pub static offset: Mutex<u64> = Mutex::new(0);

pub fn init() {
    let res = HHDM_REQUEST.get_response().unwrap();
    *offset.lock() = res.offset();
    let heap_address = phys_to_virt(0x10000);
    unsafe {
        ALLOCATOR.init(heap_address as usize, 8 * 1024 * 1024);
    }
}

pub fn phys_to_virt(addr: u64) -> u64 {
    addr + *offset.lock()
}

pub fn virt_to_phys(addr: u64) -> u64 {
    addr - *offset.lock()
}

pub fn convert_unit(size: u64) -> (f32, &'static str) {
    let mut tf = size as f32;
    let mut level = 0;
    while tf > 1024.0  {
        tf /= 1024.0;
        level += 1;
    }
    (tf, ["B", "KiB", "MiB", "GiB"][level])
}
