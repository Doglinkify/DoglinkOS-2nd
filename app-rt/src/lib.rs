#![no_std]

pub fn sys_test() {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0
        );
    }
}

pub fn sys_write(fd: usize, buf: &str) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 1,
            in("rdi") fd,
            in("rsi") buf.as_ptr(),
            in("rcx") buf.len(),
        );
    }
}

pub fn sys_fork() -> usize {
    let fork_result: usize;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 2,
            out("rcx") fork_result,
        );
    }
    fork_result
}

pub fn sys_exec(path: &str) -> ! {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 3,
            in("rdi") path.as_ptr(),
            in("rcx") path.len(),
        );
    }
    unreachable!();
}

pub fn sys_exit() -> ! {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 4,
        );
        unreachable!();
    }
}
