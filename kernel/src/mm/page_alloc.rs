use limine::request::MemoryMapRequest;
use crate::println;
use crate::mm::bitmap::Bitmap;
use super::convert_unit;
use super::phys_to_virt;
use spin::{Mutex, Lazy};

#[used]
#[link_section = ".requests"]
static MMAP_REQUEST: MemoryMapRequest = MemoryMapRequest::new();

static ALLOCATOR_STATE: Lazy<Mutex<Bitmap>> = Lazy::new(|| {
    let res = MMAP_REQUEST.get_response().unwrap();

    let usable_mem = res
        .entries()
        .iter()
        .filter(|e| e.entry_type == limine::memory_map::EntryType::USABLE);

    let max_address = usable_mem
        .clone()
        .last()
        .map(|e| e.base + e.length).unwrap();

    let conv_res = convert_unit(max_address);
    let total_pages = max_address / 4096;
    println!("[DEBUG] mm: need to manage {} pages (aka {} {})", total_pages, conv_res.0, conv_res.1);

    let bitmap_size = total_pages.div_ceil(8); // unit: bytes
    let conv_res = convert_unit(bitmap_size);
    println!("[DEBUG] mm: need bitmap size of {} {}", conv_res.0, conv_res.1);

    let bitmap_address = usable_mem
        .clone()
        .find(|region| region.length >= bitmap_size as u64)
        .map(|region| region.base)
        .unwrap();

    let bitmap_buffer = unsafe {
        core::slice::from_raw_parts_mut(phys_to_virt(bitmap_address) as *mut usize, (bitmap_size.div_ceil(8)) as usize)
    };

    let mut bitmap = Bitmap::new(bitmap_buffer);

    for region in usable_mem.clone() {
        let start_page = region.base / 4096;
        let end_page = start_page + region.length / 4096;
        bitmap.set_range(start_page as usize, end_page as usize, true);
    }

    let bitmap_start_page = bitmap_address / 4096;
    let bitmap_end_page = bitmap_start_page + bitmap_size.div_ceil(4096);
    bitmap.set_range(bitmap_start_page as usize, bitmap_end_page as usize, false);

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
    ALLOCATOR_STATE.lock().set(index as usize, true);
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
