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

fn read_word(buf: &mut [u8]) -> usize {
    for i in 0..buf.len() {
        match dlos_app_rt::sys_read() {
            b' ' | b'\n' => return i,
            c => buf[i] = c,
        }
    }
    buf.len()
}

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    dlos_app_rt::sys_write(0, "Hello, ELF!\n");
    dlos_app_rt::sys_write(1, "Please input two words: ");
    let mut buf1 = [0u8; 64];
    let mut buf2 = [0u8; 64];
    let res1 = read_word(&mut buf1);
    let res2 = read_word(&mut buf2);
    let ref1 = str::from_utf8(&buf1[..res1]).unwrap();
    let ref2 = str::from_utf8(&buf2[..res2]).unwrap();
    dlos_app_rt::sys_write(1, "Result: ");
    dlos_app_rt::sys_write(1, ref2);
    dlos_app_rt::sys_write(1, " and ");
    dlos_app_rt::sys_write(1, ref1);
    dlos_app_rt::sys_write(1, "\n");
    if dlos_app_rt::sys_fork() == 0 {
        // child
        TEST.t.set(5);
    } else {
        // parent
        TEST.t.set(4);
    }
    if TEST.t.get() == 5  {
        dlos_app_rt::sys_write(0, "Now TEST is 5!\n");
        dlos_app_rt::sys_exec("/exiter");
    }
    dlos_app_rt::sys_write(0, "Now TEST is 4!\n");
    dlos_app_rt::sys_exec("/exiter");
}
