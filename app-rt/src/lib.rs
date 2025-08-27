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

pub fn sys_exec(path: &str) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 3,
            in("rdi") path.as_ptr(),
            in("rcx") path.len(),
        );
    }
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

fn raw_sys_read() -> u8 {
    let result: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 5,
            out("rcx") result,
        );
    }
    result as u8
}

pub fn sys_read() -> u8 {
    let mut ch = raw_sys_read();
    while ch == 0xff {
        ch = raw_sys_read();
    }
    ch
}

pub fn sys_waitpid(pid: usize) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 8,
            in("rdi") pid,
        );
    }
}

pub fn sys_getpid() -> usize {
    unsafe {
        let res;
        core::arch::asm!(
            "int 0x80",
            in("rax") 9,
            out("rcx") res,
        );
        res
    }
}

pub fn sys_getticks() -> usize {
    unsafe {
        let res;
        core::arch::asm!(
            "int 0x80",
            in("rax") 10,
            out("rcx") res,
        );
        res
    }
}

pub fn sys_info(tp: u64) -> Option<usize> {
    unsafe {
        let res;
        core::arch::asm!(
            "int 0x80",
            in("rax") 11,
            in("rdi") tp,
            out("rcx") res,
        );
        match res {
            usize::MAX => None,
            v => Some(v),
        }
    }
}

pub fn sys_open(name: &str, do_create: bool) -> Option<usize> {
    unsafe {
        let res;
        core::arch::asm!(
            "int 0x80",
            in("rax") 12,
            in("rdi") name.as_ptr(),
            in("rcx") name.len(),
            in("r10") do_create as usize,
            out("rsi") res,
        );
        match res {
            usize::MAX => None,
            v => Some(v),
        }
    }
}

pub fn sys_read2(fd: usize, buf: &mut [u8]) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 13,
            in("rsi") fd,
            in("rdi") buf.as_mut_ptr(),
            in("rcx") buf.len(),
        );
    }
}

pub const SEEK_CUR: usize = 0;
pub const SEEK_END: usize = 1;
pub const SEEK_SET: usize = 2;

pub fn sys_seek(fd: usize, offset: isize, from: usize) -> usize {
    unsafe {
        let res;
        core::arch::asm!(
            "int 0x80",
            in("rax") 14,
            in("rsi") fd,
            in("rdi") from,
            in("rcx") offset,
            out("r10") res,
        );
        res
    }
}

pub fn sys_close(fd: usize) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 15,
            in("rsi") fd,
        );
    }
}

pub fn sys_remove(name: &str) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 16,
            in("rdi") name.as_ptr(),
            in("rcx") name.len(),
        );
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

struct Stdout;

impl core::fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        sys_write(1, s);
        Ok(())
    }
}

pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    let _ = Stdout.write_fmt(args);
}

#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => ($crate::_eprint(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! eprintln {
    () => ($crate::eprint!("\n"));
    ($($arg:tt)*) => ($crate::eprint!("{}\n", format_args!($($arg)*)));
}

struct Stderr;

impl core::fmt::Write for Stderr {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        sys_write(0, s);
        Ok(())
    }
}

pub fn _eprint(args: core::fmt::Arguments) {
    use core::fmt::Write;
    let _ = Stderr.write_fmt(args);
}

#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    eprint!("error: program panicked");
    if let Some(location) = info.location() {
        eprint!(" at file {} line {}", location.file(), location.line());
    }
    eprintln!(": {}", info.message());
    sys_exit();
}
