use crate::mm::page_alloc::alloc_physical_page;
use crate::mm::phys_to_virt;
use core::cmp::max;
use core::sync::atomic::Ordering;
use fatfs::Read;
use spin::Lazy;
use spin::Mutex;
use x86_64::addr::PhysAddr;
use x86_64::addr::VirtAddr;
use x86_64::registers::control::Cr3;
use x86_64::registers::control::Cr3Flags;
use x86_64::structures::paging::frame::PhysFrame;
use x86_64::structures::paging::mapper::OffsetPageTable;
use x86_64::structures::paging::page::Page;
use x86_64::structures::paging::page::Size4KiB;
use x86_64::structures::paging::page_table::PageTable;
use x86_64::structures::paging::page_table::PageTableFlags;
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
    pub fpu_state: [u128; 32],
    pub tm: u64,
    pub fs: VirtAddr,
    pub brk: u64,
}

pub static original_kernel_cr3: Lazy<(PhysFrame, Cr3Flags)> = Lazy::new(Cr3::read);

const FPU_INIT: [u128; 32] = [
    0x037fu128,
    0x1f800000000000000000u128,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
];

impl Process<'_> {
    pub fn task_0() -> Self {
        Process {
            page_table: Self::t0_p4_table(),
            context: ProcessContext::default(),
            fpu_state: FPU_INIT,
            tm: 10,
            fs: VirtAddr::new(0),
            brk: 0,
        }
    }

    fn t0_p4_table() -> OffsetPageTable<'static> {
        let p4t_pa = alloc_physical_page().unwrap();
        let p4t_va = phys_to_virt(p4t_pa);
        let p4t = unsafe { &mut *(p4t_va as *mut PageTable) };
        let kernel_p4t = unsafe {
            &mut *(phys_to_virt(original_kernel_cr3.0.start_address().as_u64()) as *mut PageTable)
        };
        Self::r_copy(kernel_p4t, p4t, 4, false, false);
        let page_table = unsafe {
            OffsetPageTable::new(p4t, x86_64::addr::VirtAddr::new_truncate(phys_to_virt(0)))
        };
        page_table
    }

    fn r_copy(
        src_table: &mut PageTable,
        dest_table: &mut PageTable,
        level: u8,
        copying_process: bool,
        is_user_page: bool,
    ) {
        // crate::println!("r_copy: src_table {:?} dest_table {:?} level {}",
        //          src_table as *const _, dest_table as *const _, level);
        dest_table.zero();
        for (index, entry) in src_table.iter_mut().enumerate() {
            if !entry.flags().contains(PageTableFlags::PRESENT) {
                continue;
            }
            if level == 1 || entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                let mut flags = entry.flags();
                if is_user_page {
                    crate::mm::page_alloc::page_incref(entry.addr().as_u64());
                    if copying_process {
                        flags.remove(PageTableFlags::WRITABLE);
                        entry.set_flags(flags);
                    }
                }
                flags.insert(PageTableFlags::USER_ACCESSIBLE);
                dest_table[index].set_addr(entry.addr(), flags);
                continue;
            }
            let new_addr = entry.addr().as_u64();
            if new_addr > (1 << 32) {
                crate::println!(
                    "[WARN] r_copy: ignoring level {level} page table at physical address 0x{:x}",
                    new_addr
                );
                continue;
            }
            let new_table_pa = alloc_physical_page().unwrap();
            let new_table_va = phys_to_virt(new_table_pa);
            let mut flags = entry.flags();
            flags.insert(PageTableFlags::USER_ACCESSIBLE);
            dest_table[index].set_addr(PhysAddr::new(new_table_pa), flags);
            let new_table = unsafe { &mut *(new_table_va as *mut PageTable) };
            let new_src = unsafe { &mut *(phys_to_virt(new_addr) as *mut PageTable) };
            if level == 4 {
                Self::r_copy(new_src, new_table, level - 1, copying_process, index < 256);
            } else {
                Self::r_copy(new_src, new_table, level - 1, copying_process, is_user_page);
            }
        }
    }

    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn copy_process(&mut self, context: *mut ProcessContext, new_tid: usize) -> Self {
        let p4t_pa = alloc_physical_page().unwrap();
        let p4t_va = phys_to_virt(p4t_pa);
        let p4t = unsafe { &mut *(p4t_va as *mut PageTable) };
        Self::r_copy(self.page_table.level_4_table_mut(), p4t, 4, true, false);
        let mut new_context = unsafe { *context };
        new_context.rcx = 0;
        unsafe {
            (*context).rcx = new_tid as u64;
        }
        Self {
            page_table: unsafe {
                OffsetPageTable::new(p4t, x86_64::addr::VirtAddr::new_truncate(phys_to_virt(0)))
            },
            context: new_context,
            fpu_state: FPU_INIT,
            tm: 0,
            fs: VirtAddr::new(0),
            brk: 0,
        }
    }

    pub fn free_page_tables(&mut self, user_only: bool) {
        let target_table = self.page_table.level_4_table_mut();
        Self::r_free(target_table, 4, user_only, false);
        if !user_only {
            crate::mm::page_alloc::dealloc_physical_page(
                target_table as *const _ as u64 - phys_to_virt(0),
            );
        }
    }

    fn r_free(target_table: &mut PageTable, level: u8, user_only: bool, is_user_page: bool) {
        let range = if level == 4 && user_only {
            0..256
        } else {
            0..512
        };
        for idx in range {
            let entry = &mut target_table[idx];
            if !entry.flags().contains(PageTableFlags::PRESENT) {
                continue;
            }
            if level == 1 || entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                if !user_only || is_user_page {
                    // when user_only is set,  only use page_decref on user pages
                    let addr = entry.addr().as_u64();
                    entry.set_unused();
                    if is_user_page {
                        crate::mm::page_alloc::page_decref(addr);
                        if crate::mm::page_alloc::page_getref(addr) == 0 {
                            //crate::println!("[DEBUG] will call dealloc_physical_page on 0x{:x}", addr);
                            crate::mm::page_alloc::dealloc_physical_page(addr);
                        }
                    }
                }
                continue;
            }
            let target_is_user_page = if level == 4 { idx < 256 } else { is_user_page };
            if !user_only || target_is_user_page {
                let new_pa = entry.addr().as_u64();
                let new_target = unsafe { &mut *(phys_to_virt(new_pa) as *mut PageTable) };
                Self::r_free(new_target, level - 1, user_only, target_is_user_page);
                entry.set_unused(); // fuck. i forgot this before
                crate::mm::page_alloc::dealloc_physical_page(new_pa);
            }
            // fuck. the above set_unused() line was written here
        }
    }
}

pub static TASKS: Mutex<[Option<Process>; 64]> = Mutex::new([const { None }; 64]);

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn do_fork(context: *mut ProcessContext) {
    static mut next_tid: usize = 0;
    unsafe {
        next_tid += 1;
    }
    let mut tasks = TASKS.lock();
    let new_process = tasks[super::sched::CURRENT_TASK_ID.load(Ordering::Relaxed)]
        .as_mut()
        .unwrap()
        .copy_process(context, unsafe { next_tid });
    tasks[unsafe { next_tid }] = Some(new_process);
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
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
    current_task.free_page_tables(true);
    current_task.context = ProcessContext::default();
    current_task.fpu_state = FPU_INIT;
    current_task.fs = VirtAddr::zero();
    current_task.brk = 0;
    let mut buf = alloc::vec![0u8; size as usize];
    elf_file.read_exact(buf.as_mut_slice()).unwrap();
    let new_elf = goblin::elf::Elf::parse(buf.as_slice());
    let new_elf = new_elf.unwrap(); // strange, but necessary
    for ph in new_elf.program_headers {
        if ph.p_type == goblin::elf::program_header::PT_LOAD {
            let start_va = VirtAddr::new_truncate(ph.p_vaddr);
            let end_va = VirtAddr::new_truncate(ph.p_vaddr + ph.p_memsz - 1);
            current_task.brk = max(current_task.brk, ph.p_vaddr + ph.p_memsz);
            // crate::println!("[DEBUG] sys_exec: {start_va:?} - {end_va:?}");
            for page in Page::range_inclusive(
                Page::<Size4KiB>::containing_address(start_va),
                Page::<Size4KiB>::containing_address(end_va),
            ) {
                let allocated_pa = alloc_physical_page().unwrap();
                unsafe {
                    let _ = current_task
                        .page_table
                        .map_to(
                            page,
                            PhysFrame::from_start_address(PhysAddr::new(allocated_pa)).unwrap(),
                            PageTableFlags::PRESENT
                                | PageTableFlags::WRITABLE
                                | PageTableFlags::USER_ACCESSIBLE,
                            &mut crate::mm::page_alloc::DLOSFrameAllocator,
                        )
                        .map(|r| {
                            r.flush();
                            crate::mm::page_alloc::page_incref(allocated_pa);
                        })
                        .map_err(|_| {
                            crate::mm::page_alloc::dealloc_physical_page(allocated_pa);
                        });
                }
            }
            let mut target_slice = unsafe {
                core::slice::from_raw_parts_mut(start_va.as_mut_ptr::<u8>(), ph.p_memsz as usize)
            };
            target_slice.fill(0u8);
            target_slice = &mut target_slice[0..(ph.p_filesz as usize)];
            target_slice.copy_from_slice(&buf[ph.file_range()]);
        }
    }
    // crate::println!("[DEBUG] will set rip to 0x{:x}", new_elf.entry);
    unsafe {
        (*args).rip = new_elf.entry;
        (*args).rsp = 0x80000000 - 80;
    }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn do_exit(args: *mut ProcessContext) {
    let c_tid = super::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
    // crate::println!("[DEBUG] task: process {c_tid} exited");
    {
        let mut tasks = TASKS.lock();
        tasks[c_tid].as_mut().unwrap().free_page_tables(false);
        tasks[c_tid] = None;
    }
    super::sched::schedule(args, true);
}
