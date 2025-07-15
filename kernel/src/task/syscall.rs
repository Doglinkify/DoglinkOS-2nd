use crate::println;
use crate::task::process::original_kernel_cr3;
use crate::task::process::ProcessContext as SyscallStackFrame;
use core::arch::naked_asm;
use core::sync::atomic::Ordering;
use x86_64::structures::idt::InterruptStackFrame;

#[unsafe(naked)]
pub extern "x86-interrupt" fn syscall_handler(_: InterruptStackFrame) {
    naked_asm!(
        "push r15",
        "push r14",
        "push r13",
        "push r12",
        "push r11",
        "push r10",
        "push r9",
        "push r8",
        "push rdi",
        "push rbp",
        "push rsi",
        "push rdx",
        "push rcx",
        "push rbx",
        "push rax",
        "mov rdi, rsp",
        "call {}",
        "pop rax",
        "pop rbx",
        "pop rcx",
        "pop rdx",
        "pop rsi",
        "pop rbp",
        "pop rdi",
        "pop r8",
        "pop r9",
        "pop r10",
        "pop r11",
        "pop r12",
        "pop r13",
        "pop r14",
        "pop r15",
        "iretq",
        sym do_syscall,
    )
}

const NUM_SYSCALLS: usize = 11;

const SYSCALL_TABLE: [fn(*mut SyscallStackFrame); NUM_SYSCALLS] = [
    sys_test,
    sys_write,
    sys_fork,
    sys_exec,
    sys_exit,
    sys_read,
    sys_setfsbase,
    sys_brk,
    sys_waitpid,
    sys_getpid,
    sys_getticks,
];

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn do_syscall(args: *mut SyscallStackFrame) {
    let call_num = unsafe { (*args).rax as usize };
    if call_num < NUM_SYSCALLS {
        if call_num == 4 {
            // sys_exit will free the current page table, so switch to original kernel page table
            unsafe {
                x86_64::registers::control::Cr3::write(
                    original_kernel_cr3.0,
                    original_kernel_cr3.1,
                );
            }
        }
        SYSCALL_TABLE[call_num](args);
        // sys_exit will call schedule() to load another page table, so we don't need to load it here
    } else {
        println!("[WARN] task/syscall: syscall {} not present", call_num);
    }
}

pub fn sys_test(_: *mut SyscallStackFrame) {
    println!("test syscall");
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sys_write(args: *mut SyscallStackFrame) {
    let (fd, ptr, size) = unsafe {
        let a = *args;
        (a.rdi, a.rsi, a.rcx)
    };
    // println!("[DEBUG] sys_write: to {fd} ptr 0x{ptr:x} size {size}");
    if fd > 1 {
        panic!("invalid fd {}", fd);
    } else {
        let mut term = crate::console::TERMINAL.lock();
        if fd == 0 {
            term.process(b"\x1b[31m");
        }
        term.process(unsafe { core::slice::from_raw_parts(ptr as *const u8, size as usize) });
        if fd == 0 {
            term.process(b"\x1b[0m");
        }
    }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sys_fork(args: *mut SyscallStackFrame) {
    super::process::do_fork(args);
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sys_exec(args: *mut SyscallStackFrame) {
    super::process::do_exec(args);
}

pub fn sys_exit(args: *mut SyscallStackFrame) {
    super::process::do_exit(args);
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sys_read(args: *mut SyscallStackFrame) {
    let res = crate::console::INPUT_BUFFER.pop().unwrap_or(0xff);
    unsafe {
        (*args).rcx = res as u64;
    }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sys_setfsbase(args: *mut SyscallStackFrame) {
    unsafe {
        // println!("sys_setfsbase called with rdi = 0x{:x}", (*args).rdi);
        use x86_64::VirtAddr;
        x86_64::registers::model_specific::FsBase::write(VirtAddr::new((*args).rdi));
        // loop{}
    }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sys_brk(args: *mut SyscallStackFrame) {
    let current = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut tasks = crate::task::process::TASKS.lock();
    let task = tasks[current].as_mut().unwrap();
    unsafe {
        (*args).rsi = task.brk;
        let tmp = (*args).rdi;
        if tmp != 0 {
            task.brk = tmp;
        }
    }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sys_waitpid(args: *mut SyscallStackFrame) {
    {
        let current = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
        let mut tasks = crate::task::process::TASKS.lock();
        let task = tasks[current].as_mut().unwrap();
        task.waiting_pid = Some(unsafe { (*args).rdi as usize });
    }
    crate::task::sched::schedule(args, false);
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sys_getpid(args: *mut SyscallStackFrame) {
    unsafe {
        (*args).rcx = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed) as u64;
    }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sys_getticks(args: *mut SyscallStackFrame) {
    unsafe {
        (*args).rcx = crate::task::sched::TOTAL_TICKS.load(Ordering::Relaxed) as u64;
    }
}
