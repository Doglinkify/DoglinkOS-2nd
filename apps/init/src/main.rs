#![no_std]
#![no_main]

#[panic_handler]
fn rust_panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Globals {
    pub t: vcell::VolatileCell<i32>
}

unsafe impl Send for Globals {}
unsafe impl Sync for Globals {}

static TEST: Globals = Globals { t: vcell::VolatileCell::new(0) };

#[unsafe(no_mangle)]
extern "C" fn _start() {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 1, // sys_write
            in("rdi") 0, // stderr
            in("rsi") "Hello, ELF!\n".as_ptr(),
            in("rcx") "Hello, ELF!\n".len(),
        );
        let fork_result: u64;
        core::arch::asm!(
            "int 0x80",
            in("rax") 2, // sys_fork
            out("rcx") fork_result,
        );
        if fork_result == 0 {
            // child
            TEST.t.set(5);
        } else {
            // parent
            TEST.t.set(4);
        }
        while TEST.t.get() == 0 {}
        core::arch::asm!(
            "int 0x80",
            in("rax") 1, // sys_write
            in("rdi") 0, // stderr
            in("rsi") if TEST.t.get() == 5 { "Now TEST is 5!\n".as_ptr() } else { "Now TEST is 4!\n".as_ptr() },
            in("rcx") "Now TEST is 5!\n".len(),
        );
        core::arch::asm!(
            "int 0x80",
            in("rax") 3, // sys_exec
            in("rdi") "/infinite-loop".as_ptr(),
            in("rcx") "/infinite-loop".len(),
        );
        unreachable!();
    }
}
