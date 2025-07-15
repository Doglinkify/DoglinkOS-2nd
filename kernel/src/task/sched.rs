use core::sync::atomic::{AtomicUsize, Ordering};
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::PhysFrame;
use x86_64::PhysAddr;

pub static CURRENT_TASK_ID: AtomicUsize = AtomicUsize::new(0);
pub static TOTAL_TICKS: AtomicUsize = AtomicUsize::new(0);

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn switch_to(
    context: *mut super::process::ProcessContext,
    next: usize,
    current_process_exited: bool,
) {
    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    // crate::println!("scheduler: switching from {current} to {next}");
    let flags = Cr3::read().1;
    let new_cr3_va;
    {
        let tasks = super::process::TASKS.lock();
        new_cr3_va = tasks[next].as_ref().unwrap().page_table.level_4_table() as *const _ as u64;
    }
    let new_cr3 =
        PhysFrame::from_start_address(PhysAddr::new(new_cr3_va - crate::mm::phys_to_virt(0)))
            .unwrap();
    unsafe {
        Cr3::write(new_cr3, flags);
    }
    {
        let mut tasks = super::process::TASKS.lock();
        if !current_process_exited {
            let cur = tasks[current].as_mut().unwrap();
            cur.context = unsafe { *context };
            cur.fs = x86_64::registers::model_specific::FsBase::read();
            unsafe {
                core::arch::x86_64::_fxsave64((&mut cur.fpu_state) as *mut _ as *mut u8);
            }
        }
        unsafe {
            let nxt = tasks[next].as_ref().unwrap();
            *context = nxt.context;
            x86_64::registers::model_specific::FsBase::write(nxt.fs);
            core::arch::x86_64::_fxrstor64((&nxt.fpu_state) as *const _ as *const u8);
        }
        CURRENT_TASK_ID.store(next, Ordering::Relaxed);
    }
}

pub extern "C" fn timer(
    context: *mut super::process::ProcessContext,
) {
    x86_64::instructions::interrupts::disable();
    schedule(context, false);
    TOTAL_TICKS.fetch_add(1, Ordering::Relaxed);
    x86_64::instructions::interrupts::enable();
    crate::apic::local::eoi();
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn schedule(
    context: *mut super::process::ProcessContext,
    current_process_exited: bool,
) {
    let mut max_tm = 0;
    let mut max_tid = 127;
    {
        let mut tasks = super::process::TASKS.lock();
        for tid in 0..64 {
            if tasks[tid].is_some() {
                let process = tasks[tid].as_ref().unwrap();
                let wait_ok = match process.waiting_pid {
                    Some(pid) => tasks[pid].is_none(),
                    None => true,
                };
                if tid != CURRENT_TASK_ID.load(Ordering::Relaxed) && process.tm > max_tm && wait_ok
                {
                    max_tm = process.tm;
                    max_tid = tid;
                }
                if wait_ok {
                    tasks[tid].as_mut().unwrap().waiting_pid = None;
                }
            }
        }
        if max_tid == 127 {
            for tid in 0..64 {
                if let Some(ref mut process) = tasks[tid] {
                    process.tm = 10;
                }
            }
            max_tid = 0;
        }
        tasks[max_tid].as_mut().unwrap().tm -= 1;
    }
    switch_to(context, max_tid, current_process_exited);
}
