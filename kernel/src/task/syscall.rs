use core::arch::naked_asm;
use crate::println;
use x86_64::structures::idt::InterruptStackFrame;
use crate::task::process::ProcessContext as SyscallStackFrame;

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

const NUM_SYSCALLS: usize = 3;

const SYSCALL_TABLE: [fn (*mut SyscallStackFrame); NUM_SYSCALLS] = [
    sys_test,
    sys_write,
    sys_fork,
];

pub extern "C" fn do_syscall(args: *mut SyscallStackFrame) {
    let call_num = unsafe { (*args).rax as usize };
    if call_num < NUM_SYSCALLS {
        SYSCALL_TABLE[call_num](args);
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
