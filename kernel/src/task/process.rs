use spin::Mutex;
use x86_64::structures::paging::mapper::OffsetPageTable;
use x86_64::structures::paging::page_table::PageTable;
use x86_64::structures::paging::page_table::PageTableFlags;
use x86_64::registers::control::Cr3;
use x86_64::addr::PhysAddr;
use crate::mm::page_alloc::alloc_physical_page;
use crate::mm::phys_to_virt;

#[derive(Default)]
#[repr(C)]
pub struct ProcessContext {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rsp: u64,
    rbp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
    rflags: u64,
}

pub struct Process<'a> {
    pub page_table: OffsetPageTable<'a>,
    pub context: ProcessContext,
}

impl<'a> Process<'a> {
    pub fn task_0() -> Self {
        Process {
            page_table: Self::new_p4_table(),
            context: ProcessContext::default(),
        }
    }

    fn new_p4_table() -> OffsetPageTable<'static> {
        let p4t_pa = alloc_physical_page().unwrap();
        let p4t_va = phys_to_virt(p4t_pa);
        let p4t = unsafe { &mut *(p4t_va as *mut PageTable) };
        let kernel_p4t = unsafe { &*(phys_to_virt(Cr3::read().0.start_address().as_u64()) as *const PageTable) };
        Self::r_copy(kernel_p4t, p4t, 4);
        let page_table = unsafe { OffsetPageTable::new(p4t, x86_64::addr::VirtAddr::new(phys_to_virt(0))) };
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
            let new_table = unsafe { &mut *(new_table_va as *mut PageTable) };
            let new_src = unsafe { &*(phys_to_virt(entry.addr().as_u64()) as *mut PageTable) };
            Self::r_copy(new_src, new_table, level - 1);
        }
    }
}

pub static TASKS: Mutex<[Option<Process>; 64]> = Mutex::new([const { None }; 64]);
