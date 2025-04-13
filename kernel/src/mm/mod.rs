pub mod bitmap;
pub mod page_alloc;
pub mod paging;

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
    {
        *offset.lock() = res.offset();
    }
    self::page_alloc::init();
    let heap_start_address = phys_to_virt(self::page_alloc::find_continuous_mem(2048));
    unsafe {
        ALLOCATOR.init(heap_start_address as usize, 8 * 1024 * 1024);
    }
}

pub fn phys_to_virt(addr: u64) -> u64 {
    addr + *offset.lock()
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
