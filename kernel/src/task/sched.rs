use core::sync::atomic::{AtomicUsize, Ordering};
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::PhysFrame;
use x86_64::PhysAddr;

pub static CURRENT_TASK_ID: AtomicUsize = AtomicUsize::new(0);

pub fn switch_to(context: *mut super::process::ProcessContext, next: usize, current_process_exited: bool) {
    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    let flags = Cr3::read().1;
    let new_cr3_va;
    {
        let tasks = super::process::TASKS.lock();
        new_cr3_va = tasks[next].as_ref().unwrap().page_table.level_4_table() as *const _ as u64;
    }
    let new_cr3 = PhysFrame::from_start_address(
        PhysAddr::new(
            new_cr3_va - crate::mm::phys_to_virt(0)
        )
    ).unwrap();
    unsafe {
        Cr3::write(new_cr3, flags);
    }
    {
        let mut tasks = super::process::TASKS.lock();
        if !current_process_exited {
            tasks[current].as_mut().unwrap().context = unsafe { *context };
        }
        unsafe {
            *context = tasks[next].as_ref().unwrap().context;
        }
        CURRENT_TASK_ID.store(next, Ordering::Relaxed);
    }
}

pub extern "C" fn schedule(context: *mut super::process::ProcessContext, current_process_exited: bool) {
    x86_64::instructions::interrupts::disable();
    let mut max_tm = 0;
    let mut max_tid = 127;
    {
        let mut tasks = super::process::TASKS.lock();
        for tid in 0..64 {
            if let Some(ref process) = tasks[tid] {
                if tid != CURRENT_TASK_ID.load(Ordering::Relaxed) {
                    if process.tm > max_tm {
                        max_tm = process.tm;
                        max_tid = tid;
                    }
                }
            }
        }
        if max_tid == 127 {
            for tid in 0..64 {
                if let Some(ref mut process) = tasks[tid] {
                    if tid != CURRENT_TASK_ID.load(Ordering::Relaxed) {
                        process.tm = 10;
                    }
                }
            }
            max_tid = 0;
        }
        tasks[max_tid].as_mut().unwrap().tm -= 1;
    }
    switch_to(context, max_tid, current_process_exited);
    x86_64::instructions::interrupts::enable();
    crate::apic::local::eoi();
}
