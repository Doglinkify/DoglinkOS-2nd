use limine::request::{HhdmRequest, MemoryMapRequest};
use spin::Mutex;
use good_memory_allocator::SpinLockedAllocator;
use crate::println;

#[used]
#[link_section = ".requests"]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[link_section = ".requests"]
static MMAP_REQUEST: MemoryMapRequest = MemoryMapRequest::new();

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

pub fn show_mmap() {
    let res = MMAP_REQUEST.get_response().unwrap();
    for entry in res.entries() {
        println!("Base: 0x{:x}, Length: 0x{:x}, Type: {}",
                 entry.base, entry.length,
                 get_entry_type_string(entry));
    }
}
