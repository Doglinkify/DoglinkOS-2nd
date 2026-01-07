use crate::println;
use crate::task::process::ProcessContext as SyscallStackFrame;
use crate::task::process::ORIGINAL_KERNEL_CR3;
use crate::vfs::SeekFrom;
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

const NUM_SYSCALLS: usize = 17;

const SYSCALL_TABLE: [fn(&mut SyscallStackFrame); NUM_SYSCALLS] = [
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
    sys_info,
    sys_open,
    sys_read2,
    sys_seek,
    sys_close,
    sys_remove,
];

unsafe extern "C" fn do_syscall(args: *mut SyscallStackFrame) {
    let call_num = unsafe { (*args).rax as usize };
    if call_num < NUM_SYSCALLS {
        if call_num == 4 {
            // sys_exit will free the current page table, so switch to original kernel page table
            unsafe {
                x86_64::registers::control::Cr3::write(
                    ORIGINAL_KERNEL_CR3.0,
                    ORIGINAL_KERNEL_CR3.1,
                );
            }
        }
        SYSCALL_TABLE[call_num](unsafe { &mut *args });
        // sys_exit will call schedule() to load another page table, so we don't need to load it here
    } else {
        println!("[WARN] task/syscall: syscall {} not present", call_num);
    }
}

pub fn sys_test(_: &mut SyscallStackFrame) {
    println!("test syscall");
}

pub fn sys_write(args: &mut SyscallStackFrame) {
    let (fd, ptr, size) = (args.rdi, args.rsi, args.rcx);
    // println!("[DEBUG] sys_write: to {fd} ptr 0x{ptr:x} size {size}");
    let current = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut tasks = crate::task::process::TASKS.lock();
    let task = tasks[current].as_mut().unwrap();
    let _ = task.files[fd as usize].as_ref().map(|file| {
        file.lock()
            .write_all(unsafe { core::slice::from_raw_parts(ptr as *const u8, size as usize) })
    });
}

pub fn sys_fork(args: &mut SyscallStackFrame) {
    super::process::do_fork(args);
}

pub fn sys_exec(args: &mut SyscallStackFrame) {
    super::process::do_exec(args);
}

pub fn sys_exit(args: &mut SyscallStackFrame) {
    super::process::do_exit(args);
}

pub fn sys_read(args: &mut SyscallStackFrame) {
    let res = crate::console::INPUT_BUFFER.pop().unwrap_or(0xff);
    args.rcx = res as u64;
}

pub fn sys_setfsbase(args: &mut SyscallStackFrame) {
    use x86_64::VirtAddr;
    x86_64::registers::model_specific::FsBase::write(VirtAddr::new(args.rdi));
}

pub fn sys_brk(args: &mut SyscallStackFrame) {
    let current = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut tasks = crate::task::process::TASKS.lock();
    let task = tasks[current].as_mut().unwrap();
    args.rsi = task.brk;
    let tmp = args.rdi;
    if tmp != 0 {
        task.brk = tmp;
    }
}

pub fn sys_waitpid(args: &mut SyscallStackFrame) {
    {
        let current = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
        let mut tasks = crate::task::process::TASKS.lock();
        let task = tasks[current].as_mut().unwrap();
        task.waiting_pid = Some(args.rdi as usize);
    }
    crate::task::sched::schedule(args, false);
}

pub fn sys_getpid(args: &mut SyscallStackFrame) {
    args.rcx = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed) as u64;
}

pub fn sys_getticks(args: &mut SyscallStackFrame) {
    args.rcx = crate::task::sched::TOTAL_TICKS.load(Ordering::Relaxed) as u64;
}

pub fn sys_info(args: &mut SyscallStackFrame) {
    args.rcx = match args.rdi {
        0 => crate::console::TERMINAL.lock().columns() as u64,
        1 => crate::console::TERMINAL.lock().rows() as u64,
        2 => crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed) as u64,
        3 => crate::task::sched::TOTAL_TICKS.load(Ordering::Relaxed) as u64,
        4 => {
            crate::console::ECHO_FLAG.store(false, Ordering::Relaxed);
            0
        }
        5 => {
            crate::console::ECHO_FLAG.store(true, Ordering::Relaxed);
            0
        }
        6 => crate::console::FRAMEBUFFER.width as u64,
        7 => crate::console::FRAMEBUFFER.height as u64,
        8 => crate::console::FRAMEBUFFER.addr as u64,
        9 => crate::console::FRAMEBUFFER.pitch as u64,
        _ => u64::MAX,
    };
}

pub fn sys_open(args: &mut SyscallStackFrame) {
    let path = unsafe { core::str::from_raw_parts(args.rdi as *const u8, args.rcx as usize) };
    let do_create = args.r10 != 0;
    let current = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut tasks = crate::task::process::TASKS.lock();
    let task = tasks[current].as_mut().unwrap();
    if let Some((res, _)) = task.files.iter().enumerate().find(|x| x.1.is_none()) {
        let tmp = if do_create {
            crate::vfs::create_file_or_open_existing(path).ok()
        } else {
            crate::vfs::get_file(path).ok()
        };
        if let Some(file) = tmp {
            task.files[res] = Some(file);
            args.rsi = res as u64;
        } else {
            args.rsi = u64::MAX;
        }
    } else {
        args.rsi = u64::MAX;
    }
}

pub fn sys_read2(args: &mut SyscallStackFrame) {
    let buf = unsafe { core::slice::from_raw_parts_mut(args.rdi as *mut u8, args.rcx as usize) };
    let current = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut tasks = crate::task::process::TASKS.lock();
    let task = tasks[current].as_mut().unwrap();
    task.files[args.rsi as usize]
        .as_ref()
        .unwrap()
        .lock()
        .read_exact(buf);
}

pub fn sys_seek(args: &mut SyscallStackFrame) {
    let pos = match args.rdi {
        0 => SeekFrom::Current(args.rcx.cast_signed() as isize),
        1 => SeekFrom::End(args.rcx.cast_signed() as isize),
        2 => SeekFrom::Start(args.rcx as usize),
        _ => return,
    };
    let current = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut tasks = crate::task::process::TASKS.lock();
    let task = tasks[current].as_mut().unwrap();
    args.r10 = task.files[args.rsi as usize]
        .as_ref()
        .unwrap()
        .lock()
        .seek(pos) as u64;
}

pub fn sys_close(args: &mut SyscallStackFrame) {
    let current = crate::task::sched::CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut tasks = crate::task::process::TASKS.lock();
    let task = tasks[current].as_mut().unwrap();
    task.files[args.rsi as usize] = None;
}

pub fn sys_remove(args: &mut SyscallStackFrame) {
    let path = unsafe { core::str::from_raw_parts(args.rdi as *const u8, args.rcx as usize) };
    crate::vfs::remove_file(path);
}
