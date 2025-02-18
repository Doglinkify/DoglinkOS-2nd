use spin::Mutex;
use x86_64::structures::paging::mapper::OffsetPageTable;
use x86_64::structures::paging::page_table::PageTable;
use x86_64::structures::paging::page_table::PageTableFlags;
use x86_64::registers::control::Cr3;
use x86_64::addr::PhysAddr;
use crate::mm::page_alloc::alloc_physical_page;
use crate::mm::phys_to_virt;
use core::sync::atomic::Ordering;
use fatfs::Read;
use x86_64::structures::paging::page::Page;
use x86_64::structures::paging::page::Size4KiB;
use x86_64::addr::VirtAddr;
use x86_64::structures::paging::frame::PhysFrame;
use x86_64::structures::paging::Mapper;

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

pub fn do_exec(args: *mut ProcessContext) {
    let path = unsafe {
        let slice = core::slice::from_raw_parts((*args).rdi as *const _, (*args).rcx as usize);
        core::str::from_utf8(slice).unwrap()
    };
    let mut elf_file = crate::vfs::get_file(path);
    let mut size = 0;
    for e in elf_file.extents() {
        size += e.unwrap().size;
    }
    let c_tid = super::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut tasks = TASKS.lock();
    let current_task = tasks[c_tid].as_mut().unwrap();
    crate::println!("[xiaoyi-DEBUG] sys_exec: the size of target ELF file is {size}");
    let mut buf = alloc::vec![0u8; size as usize];
    elf_file.read_exact(buf.as_mut_slice()).unwrap();
    {
        let mut hash: u64 = 0;
        for byte in &buf {
            hash = hash * 3131 + *byte as u64;
        }
        crate::println!("[xiaoyi-DEBUG] hash of the ELF is 0x{:016x}", hash);
    }
    let new_elf = goblin::elf::Elf::parse(buf.as_slice()).unwrap();
    for ph in new_elf.program_headers {
        if ph.p_type == goblin::elf::program_header::PT_LOAD {
            let start_va = VirtAddr::new(ph.p_vaddr);
            let end_va = VirtAddr::new(ph.p_vaddr + ph.p_memsz);
            for page in Page::range_inclusive(Page::<Size4KiB>::containing_address(start_va), Page::<Size4KiB>::containing_address(end_va)) {
                let allocated_pa = PhysAddr::new(alloc_physical_page().unwrap());
                unsafe {
                    current_task.page_table.map_to(
                        page,
                        PhysFrame::from_start_address(allocated_pa).unwrap(),
                        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
                        &mut crate::mm::page_alloc::DLOSFrameAllocator,
                    ).unwrap().flush();
                }
                //crate::println!("[DEBUG] sys_exec: mapped {:?} to {:?}", allocated_pa, page);
            }
            let mut target_slice = unsafe {
                core::slice::from_raw_parts_mut(start_va.as_mut_ptr::<u8>(), ph.p_memsz as usize)
            };
            target_slice.fill(0u8);
            target_slice = &mut target_slice[0..(ph.p_filesz as usize)];
            target_slice.copy_from_slice(&buf[ph.file_range()]);
            //crate::println!("[DEBUG] sys_exec: copied {:?} to {:?}", &buf[ph.file_range()] as *const _, target_slice as *const _);
        }
    }
    unsafe {
        (*args).rip = new_elf.entry;
    }
}
