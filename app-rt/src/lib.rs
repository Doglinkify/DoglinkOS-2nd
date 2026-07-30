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

pub fn sys_read3(fd: usize, buf: &mut [u8]) -> usize {
    unsafe {
        let res;
        core::arch::asm!(
            "int 0x80",
            in("rax") 22,
            in("rsi") fd,
            in("rdi") buf.as_mut_ptr(),
            in("rcx") buf.len(),
            out("r10") res,
        );
        res
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

pub fn sys_mount(typ: usize, disk: usize, part: usize, mountpoint: &str) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 17,
            in("rdi") mountpoint.as_ptr(),
            in("rcx") mountpoint.len(),
            in("rsi") typ,
            in("rdx") disk,
            in("r9") part,
        );
    }
}

pub const DIRENT_NAME_CAP: usize = 255;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct DirEntry {
    pub is_dir: u8,
    pub name: [u8; DIRENT_NAME_CAP],
}

impl DirEntry {
    pub const fn empty() -> Self {
        Self {
            is_dir: 0,
            name: [0; DIRENT_NAME_CAP],
        }
    }

    pub fn name(&self) -> &str {
        let len = self
            .name
            .iter()
            .position(|&x| x == 0)
            .unwrap_or(DIRENT_NAME_CAP);
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }

    pub fn is_dir(&self) -> bool {
        self.is_dir != 0
    }
}

pub fn sys_opendir(path: &str) -> Option<usize> {
    unsafe {
        let res;
        core::arch::asm!(
            "int 0x80",
            in("rax") 18,
            in("rdi") path.as_ptr(),
            in("rcx") path.len(),
            out("rsi") res,
        );
        match res {
            usize::MAX => None,
            v => Some(v),
        }
    }
}

pub fn sys_getdents(fd: usize, entries: &mut [DirEntry]) -> Option<usize> {
    unsafe {
        let res;
        core::arch::asm!(
            "int 0x80",
            in("rax") 19,
            in("rsi") fd,
            in("rdi") entries.as_mut_ptr(),
            in("rcx") entries.len(),
            out("r10") res,
        );
        match res {
            usize::MAX => None,
            v => Some(v),
        }
    }
}

pub fn sys_closedir(fd: usize) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 20,
            in("rsi") fd,
        );
    }
}

pub const IPC_FLAG_NONBLOCK: usize = 1;

pub const IPC_CMD_CREATE: usize = 0;
pub const IPC_CMD_SEND: usize = 1;
pub const IPC_CMD_RECV: usize = 2;
pub const IPC_CMD_CLOSE: usize = 3;
pub const IPC_CMD_DUP: usize = 4;
pub const IPC_CMD_BIND: usize = 5;
pub const IPC_CMD_CONNECT: usize = 6;
pub const IPC_CMD_ACCEPT: usize = 7;

pub fn sys_ipc(
    cmd: usize,
    arg0: usize,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 21,
            in("rdi") cmd,
            in("rsi") arg0,
            in("rdx") arg1,
            in("rcx") arg2,
            in("r8") arg3,
            in("r9") arg4,
            lateout("rax") ret,
        );
    }
    ret
}

pub fn sys_ipc_create(flags: usize) -> Option<(usize, usize)> {
    let left: isize;
    let right: usize;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 21,
            in("rdi") IPC_CMD_CREATE,
            in("rsi") flags,
            lateout("rax") left,
            lateout("rdx") right,
        );
    }
    if left < 0 {
        None
    } else {
        Some((left as usize, right))
    }
}

pub fn sys_ipc_send(handle: usize, buf: &[u8], flags: usize) -> isize {
    sys_ipc(
        IPC_CMD_SEND,
        handle,
        buf.as_ptr() as usize,
        buf.len(),
        flags,
        0,
    )
}

pub fn sys_ipc_recv(handle: usize, buf: &mut [u8], flags: usize) -> isize {
    sys_ipc(
        IPC_CMD_RECV,
        handle,
        buf.as_mut_ptr() as usize,
        buf.len(),
        flags,
        0,
    )
}

pub fn sys_ipc_close(handle: usize) -> isize {
    sys_ipc(IPC_CMD_CLOSE, handle, 0, 0, 0, 0)
}

pub fn sys_ipc_dup(handle: usize) -> Option<usize> {
    let res = sys_ipc(IPC_CMD_DUP, handle, 0, 0, 0, 0);
    if res < 0 { None } else { Some(res as usize) }
}

pub fn sys_ipc_bind(name: &str) -> Option<usize> {
    let res = sys_ipc(IPC_CMD_BIND, name.as_ptr() as usize, name.len(), 0, 0, 0);
    if res < 0 { None } else { Some(res as usize) }
}

pub fn sys_ipc_connect(name: &str) -> Option<usize> {
    let res = sys_ipc(IPC_CMD_CONNECT, name.as_ptr() as usize, name.len(), 0, 0, 0);
    if res < 0 { None } else { Some(res as usize) }
}

pub fn sys_ipc_accept(handle: usize) -> Option<usize> {
    let res = sys_ipc(IPC_CMD_ACCEPT, handle, 0, 0, 0, 0);
    if res < 0 { None } else { Some(res as usize) }
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
