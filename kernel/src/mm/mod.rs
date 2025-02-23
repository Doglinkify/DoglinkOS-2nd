pub mod bitmap;
pub mod page_alloc;
pub mod paging;

use limine::request::HhdmRequest;
use spin::Mutex;
use good_memory_allocator::SpinLockedAllocator;
use x86_64::structures::paging::page::Page;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::page_table::PageTable;
use x86_64::structures::paging::page_table::PageTableFlags;
use x86_64::structures::paging::OffsetPageTable;
use x86_64::addr::VirtAddr;
use x86_64::structures::paging::Mapper;
use x86_64::structures::paging::FrameAllocator;

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
    let heap_start_address = 0xffff800100000000; // upper bound of HHDM
    let heap_end_address = 0xffff800100800000; // 8 MiB heap
    let p4tt = unsafe { &mut *(phys_to_virt(Cr3::read().0.start_address().as_u64()) as *mut PageTable) };
    let mut page_table = unsafe { OffsetPageTable::new(p4tt, x86_64::addr::VirtAddr::new(phys_to_virt(0))) };
    for page in Page::range_inclusive(
        Page::containing_address(VirtAddr::new(heap_start_address)),
        Page::containing_address(VirtAddr::new(heap_end_address - 1)),
    ) {
        let mut fa = self::page_alloc::DLOSFrameAllocator;
        let frame = fa.allocate_frame().unwrap();
        unsafe {
            page_table.map_to(
                page,
                frame,
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
                &mut fa,
            ).unwrap().flush();
        }
    }
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
