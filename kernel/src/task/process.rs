use spin::Lazy;
use x86_64::structures::paging::mapper::OffsetPageTable;
use x86_64::structures::paging::page_table::PageTable;
use x86_64::structures::paging::page_table::PageTableFlags;
use x86_64::registers::control::Cr3;
use x86_64::addr::PhysAddr;
use crate::mm::page_alloc::alloc_physical_page;
use crate::mm::phys_to_virt;

pub struct Process<'a> {
    pub page_table: Option<OffsetPageTable<'a>>,
    pub present: bool,
}

pub static TASKS: Lazy<[Process; 64]> = Lazy::new(|| {
    let mut tasks = core::array::from_fn(|_| Process {page_table: None, present: false});
    tasks[0].present = true;
    tasks[0].page_table = Some(new_p4_table());
    tasks
});

pub fn new_p4_table() -> OffsetPageTable<'static> {
    crate::println!("new_p4_table() called");
    let p4t_pa = alloc_physical_page().unwrap();
    let p4t_va = phys_to_virt(p4t_pa);
    let mut p4t = unsafe { &mut *(p4t_va as *mut PageTable) };
    let kernel_p4t = unsafe { &*(phys_to_virt(Cr3::read().0.start_address().as_u64()) as *const PageTable) };
    r_copy(kernel_p4t, p4t, 4);
    crate::println!("r_copy returns");
    let mut page_table = unsafe { OffsetPageTable::new(p4t, x86_64::addr::VirtAddr::new(phys_to_virt(0))) };
    page_table
}

fn r_copy(src_table: &PageTable, dest_table: &mut PageTable, level: u8) {
    for (index, entry) in src_table.iter().enumerate() {
        if (level == 1)
            || entry.is_unused()
            || entry.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            let mut flags = entry.flags();
            flags.insert(PageTableFlags::USER_ACCESSIBLE);
            dest_table[index].set_addr(entry.addr(), flags);
            continue;
        }
        let new_table_pa = alloc_physical_page().unwrap();
        let new_table_va = phys_to_virt(new_table_pa);
        let mut flags = entry.flags();
        flags.insert(PageTableFlags::USER_ACCESSIBLE);
        dest_table[index].set_addr(PhysAddr::new(new_table_pa), flags);
        let mut new_table = unsafe { &mut *(new_table_va as *mut PageTable) };
        let new_src = unsafe { &*(phys_to_virt(entry.addr().as_u64()) as *mut PageTable) };
        r_copy(new_src, new_table, level - 1);
    }
}
