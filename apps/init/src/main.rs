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
extern "C" fn _start() -> ! {
    dlos_app_rt::sys_write(0, "Hello, ELF!\n");
    if dlos_app_rt::sys_fork() == 0 {
        // child
        TEST.t.set(5);
    } else {
        // parent
        TEST.t.set(4);
    }
    dlos_app_rt::sys_write(0, if TEST.t.get() == 5 { "Now TEST is 5!\n" } else { "Now TEST is 4!\n" });
    dlos_app_rt::sys_exec("/exiter");
}
