use limine::request::MemoryMapRequest;
use crate::println;
use crate::mm::bitmap::PageMan;
use super::phys_to_virt;
use spin::{Mutex, Lazy};
use x86_64::structures::paging::FrameAllocator;
use x86_64::structures::paging::FrameDeallocator;
use x86_64::addr::PhysAddr;
use x86_64::structures::paging::PhysFrame;
use x86_64::structures::paging::Size4KiB;
use x86_64::structures::idt::PageFaultErrorCode;
use x86_64::registers::control::Cr2;
use x86_64::structures::paging::page::Page;
use x86_64::structures::paging::Mapper;
use x86_64::structures::paging::PageTableFlags;

#[used]
#[link_section = ".requests"]
static MMAP_REQUEST: MemoryMapRequest = MemoryMapRequest::new();

pub static ALLOCATOR_STATE: Lazy<Mutex<PageMan>> = Lazy::new(|| {
    let res = MMAP_REQUEST.get_response().unwrap();

    let usable_mem = res
        .entries()
        .iter()
        .filter(|e| e.entry_type == limine::memory_map::EntryType::USABLE);

    let max_address = usable_mem
        .clone()
        .last()
        .map(|e| e.base + e.length).unwrap();

    // let conv_res = convert_unit(max_address);
    let total_pages = max_address / 4096;
    // println!("[DEBUG] mm: need to manage {} pages (aka {} {})", total_pages, conv_res.0, conv_res.1);

    let bitmap_size = PageMan::calc_size(total_pages); // unit: (count, count, bytes)
    // let conv_res = convert_unit(bitmap_size.2);
    // println!("[DEBUG] mm: need bitmap size of {} {}", conv_res.0, conv_res.1);

    let bitmap_address = usable_mem
        .clone()
        .find(|region| region.length >= bitmap_size.2)
        .map(|region| region.base)
        .unwrap();

    let bitmap_buffer1 = unsafe {
        core::slice::from_raw_parts_mut(phys_to_virt(bitmap_address) as *mut usize, bitmap_size.0 as usize)
    };

    let bitmap_buffer2 = unsafe {
        core::slice::from_raw_parts_mut(phys_to_virt(bitmap_address + bitmap_size.0 * 8) as *mut u8, bitmap_size.1 as usize)
    };

    // println!("[DEBUG] mm: bitmap_buffer1 is {:?}", bitmap_buffer1.as_ptr());
    // println!("[DEBUG] mm: bitmap_buffer2 is {:?}", bitmap_buffer2.as_ptr());

    let mut bitmap = PageMan::new(bitmap_buffer1, bitmap_buffer2);

    for region in usable_mem.clone() {
        let start_page = region.base / 4096;
        let end_page = start_page + region.length / 4096;
        bitmap.set_range(start_page as usize, end_page as usize, true);
    }

    let bitmap_start_page = bitmap_address / 4096;
    let bitmap_end_page = bitmap_start_page + bitmap_size.2.div_ceil(4096);
    bitmap.set_range(bitmap_start_page as usize, bitmap_end_page as usize, false);

    // println!("[DEBUG] mm: bitmap_end_page is 0x{:x}", bitmap_end_page * 4096);

    Mutex::new(bitmap)
});

// reserved for future use
pub fn get_entry_type_string(entry: &limine::memory_map::Entry) -> &str {
    match entry.entry_type {
        limine::memory_map::EntryType::USABLE => {"USABLE"},
        limine::memory_map::EntryType::RESERVED => {"RESERVED"},
        limine::memory_map::EntryType::ACPI_RECLAIMABLE => {"ACPI_RECLAIMABLE"},
        limine::memory_map::EntryType::ACPI_NVS => {"ACPI_NVS"},
        limine::memory_map::EntryType::BAD_MEMORY => {"BAD_MEMORY"},
        limine::memory_map::EntryType::BOOTLOADER_RECLAIMABLE => {"BOOTLOADER_RECLAIMABLE"},
        limine::memory_map::EntryType::KERNEL_AND_MODULES => {"KERNEL_AND_MODULES"},
        limine::memory_map::EntryType::FRAMEBUFFER => {"FRAMEBUFFER"},
        _ => {"UNK"}
    }
}

pub fn init() {
    Lazy::force(&ALLOCATOR_STATE);
}

pub fn find_continuous_mem(cnt: usize) -> u64 {
    let mut current_size = 0;
    let mut state = ALLOCATOR_STATE.lock();
    let limit = state.len();
    for i in 0..limit {
        if state.get(i) {
            current_size += 1;
        } else {
            if current_size == cnt {
                state.set_range(i - cnt, i, false);
                return ((i - cnt) * 4096) as u64;
            } else {
                current_size = 0;
            }
        }
    }
    0
}

pub fn alloc_physical_page() -> Option<u64> {
    let mut allocator_state = ALLOCATOR_STATE.lock();
    for i in 0..allocator_state.len() {
        if allocator_state.get(i) {
            allocator_state.set(i, false);
            return Some((i * 4096) as u64);
        }
    }
    None
}

pub fn dealloc_physical_page(addr: u64) {
    let index = addr / 4096;
    let mut alc = ALLOCATOR_STATE.lock();
    if alc.get(index as usize) {
        println!("[WRANING] mm: detected double free on page 0x{addr}, kernel bug?");
    }
    alc.set(index as usize, true);
}

pub fn page_incref(addr: u64) {
    ALLOCATOR_STATE.lock().incref(addr as usize / 4096);
}

pub fn page_decref(addr: u64) {
    ALLOCATOR_STATE.lock().decref(addr as usize / 4096);
}

pub fn page_getref(addr: u64) -> u8 {
    ALLOCATOR_STATE.lock().getref(addr as usize / 4096)
}

pub struct DLOSFrameAllocator;

unsafe impl FrameAllocator<Size4KiB> for DLOSFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        Some(
            PhysFrame::from_start_address(
                PhysAddr::new(
                    alloc_physical_page().unwrap()
                )
            ).unwrap()
        )
    }
}

pub struct DLOSFrameDeallocator;

impl FrameDeallocator<Size4KiB> for DLOSFrameDeallocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        dealloc_physical_page(frame.start_address().as_u64());
    }
}

pub fn test() {
    let mut addresses = [0u64; 10];
    for i in 0..10 {
        addresses[i] = alloc_physical_page().unwrap();
        println!("[DEBUG] page_alloc: Allocation #1-{}: 0x{:x}", i, addresses[i]);
    }
    for i in 0..10 {
        dealloc_physical_page(addresses[i]);
    }
    for i in 0..10 {
        addresses[i] = alloc_physical_page().unwrap();
        println!("[DEBUG] page_alloc: Allocation #2-{}: 0x{:x}", i, addresses[i]);
    }
    for i in 0..10 {
        dealloc_physical_page(addresses[i]);
    }
}

pub fn do_user_page_fault(code: PageFaultErrorCode) {
    let addr = Cr2::read().unwrap();
    let page = Page::<Size4KiB>::containing_address(addr);
    let current = crate::task::sched::CURRENT_TASK_ID.load(core::sync::atomic::Ordering::Relaxed);
    let mut tasks = crate::task::process::TASKS.lock();
    let pgt = &mut tasks[current]
        .as_mut().unwrap()
        .page_table;
    if code.contains(PageFaultErrorCode::PROTECTION_VIOLATION | PageFaultErrorCode::CAUSED_BY_WRITE) {
        let phys_addr = pgt
            .translate_page(page).unwrap()
            .start_address().as_u64();
        if page_getref(phys_addr) > 1 {
            let new_page_pa = alloc_physical_page().unwrap();
            let new_page_va = super::phys_to_virt(new_page_pa);
            let old_page_va = addr.align_down(4096u64);
            unsafe {
                core::ptr::copy(old_page_va.as_ptr::<u8>(), new_page_va as *mut u8, 4096);
                page_decref(phys_addr);
                pgt.unmap(page).unwrap().1.flush();
                pgt.map_to(
                    page,
                    PhysFrame::from_start_address(PhysAddr::new(new_page_pa)).unwrap(),
                    PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
                    &mut DLOSFrameAllocator,
                ).unwrap().flush();
                page_incref(new_page_pa);
            }
        } else {
            unsafe {
                pgt.update_flags(page, PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE).unwrap().flush();
            }
        }
    } else if within_stack_range(addr) {
        let new_page_pa = alloc_physical_page().unwrap();
        page_incref(new_page_pa);
        unsafe {
            pgt.map_to(
                page,
                PhysFrame::from_start_address(PhysAddr::new(new_page_pa)).unwrap(),
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
                &mut DLOSFrameAllocator,
            ).unwrap().flush();
        }
    } else {
        panic!("unrecoverable user page fault");
    }
}

fn within_stack_range(addr: x86_64::VirtAddr) -> bool {
    let ua = addr.as_u64();
    (0x7fe00000..0x80000000).contains(&ua)
}
