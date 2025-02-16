use spin::Mutex;
use x86_64::structures::paging::mapper::OffsetPageTable;
use x86_64::structures::paging::page_table::PageTable;
use x86_64::structures::paging::page_table::PageTableFlags;
use x86_64::registers::control::Cr3;
use x86_64::addr::PhysAddr;
use crate::mm::page_alloc::alloc_physical_page;
use crate::mm::phys_to_virt;
use core::sync::atomic::Ordering;

#[derive(Default, Copy, Clone, Debug)]
#[repr(C)]
pub struct ProcessContext {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    // InterruptStackFrameValue
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

pub struct Process<'a> {
    pub page_table: OffsetPageTable<'a>,
    pub context: ProcessContext,
    pub tm: u64,
}

impl<'a> Process<'a> {
    pub fn task_0() -> Self {
        Process {
            page_table: Self::new_p4_table(),
            context: ProcessContext::default(),
            tm: 0,
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
        // crate::println!("r_copy: src_table {:?} dest_table {:?} level {}",
        //          src_table as *const _, dest_table as *const _, level);
        dest_table.zero();
        for (index, entry) in src_table.iter().enumerate() {
            if !entry.flags().contains(PageTableFlags::PRESENT) {
                continue;
            }
            if level == 1 || entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                let mut flags = entry.flags();
                flags.insert(PageTableFlags::USER_ACCESSIBLE);
                dest_table[index].set_addr(entry.addr(), flags);
                continue;
            }
            let new_addr = entry.addr().as_u64();
            if new_addr > (1 << 32) {
                crate::println!("[WARN] r_copy: ignoring level {level} page table at physical address 0x{:x}", new_addr);
                continue;
            }
            let new_table_pa = alloc_physical_page().unwrap();
            let new_table_va = phys_to_virt(new_table_pa);
            let mut flags = entry.flags();
            flags.insert(PageTableFlags::USER_ACCESSIBLE);
            dest_table[index].set_addr(PhysAddr::new(new_table_pa), flags);
            let new_table = unsafe { &mut *(new_table_va as *mut PageTable) };
            let new_src = unsafe { &*(phys_to_virt(new_addr) as *mut PageTable) };
            Self::r_copy(new_src, new_table, level - 1);
        }
    }

    pub fn copy_process(&self, context: *mut ProcessContext, new_tid: usize) -> Self {
        let p4t_pa = alloc_physical_page().unwrap();
        let p4t_va = phys_to_virt(p4t_pa);
        let p4t = unsafe { &mut *(p4t_va as *mut PageTable) };
        Self::r_copy(self.page_table.level_4_table(), p4t, 4);
        let mut new_context = unsafe { *context };
        new_context.rcx = 0;
        unsafe {
            (*context).rcx = new_tid as u64;
        }
        Self {
            page_table: unsafe { OffsetPageTable::new(p4t, x86_64::addr::VirtAddr::new(phys_to_virt(0))) },
            context: new_context,
            tm: 0,
        }
    }
}

pub static TASKS: Mutex<[Option<Process>; 64]> = Mutex::new([const { None }; 64]);

pub fn do_fork(context: *mut ProcessContext) {
    static mut next_tid: usize = 0;
    unsafe {
        next_tid += 1;
    }
    let mut tasks = TASKS.lock();
    let new_process = tasks[super::sched::CURRENT_TASK_ID.load(Ordering::Relaxed)].as_ref().unwrap().copy_process(context, unsafe { next_tid });
    tasks[unsafe { next_tid }] = Some(new_process);
}
