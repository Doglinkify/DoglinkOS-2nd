use core::arch::naked_asm;
use crate::println;
use x86_64::structures::idt::InterruptStackFrame;
use crate::task::process::ProcessContext as SyscallStackFrame;
use crate::task::process::original_kernel_cr3;

#[naked]
pub extern "x86-interrupt" fn syscall_handler(_: InterruptStackFrame) {
    unsafe {
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
}

const NUM_SYSCALLS: usize = 5;

const SYSCALL_TABLE: [fn (*mut SyscallStackFrame); NUM_SYSCALLS] = [
    sys_test,
    sys_write,
    sys_fork,
    sys_exec,
    sys_exit,
];

pub extern "C" fn do_syscall(args: *mut SyscallStackFrame) {
    let call_num = unsafe { (*args).rax as usize };
    if call_num < NUM_SYSCALLS {
        if call_num == 4 { // sys_exit will free the current page table, so switch to original kernel page table
            unsafe {
                x86_64::registers::control::Cr3::write(original_kernel_cr3.0, original_kernel_cr3.1);
            }
        }
        SYSCALL_TABLE[call_num](args);
        // sys_exit will call schedule() to load another page table, so we don't need to load it here
    } else {
        panic!("syscall {} not present", call_num);
    }
}

pub fn sys_test(_: *mut SyscallStackFrame) {
    println!("test syscall");
}

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
        term.process(unsafe {
            core::slice::from_raw_parts(ptr as *const u8, size as usize)
        });
        if fd == 0 {
            term.process(b"\x1b[0m");
        }
    }
}

pub fn sys_fork(args: *mut SyscallStackFrame) {
    super::process::do_fork(args);
}

pub fn sys_exec(args: *mut SyscallStackFrame) {
    super::process::do_exec(args);
}

pub fn sys_exit(args: *mut SyscallStackFrame) {
    super::process::do_exit(args);
}
