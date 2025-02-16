#![no_std]
#![no_main]

#[panic_handler]
fn rust_panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn fib(n: i32) -> i32 {
    if n < 2 {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

fn i32_to_str(mut x: i32, buf: &mut [u8]) -> usize {
    let mut len: usize = 0;
    while x > 0 {
        buf[len] = (x % 10) as u8 + 0x30;
        x /= 10;
        len += 1;
    }
    for i in 0..(len / 2) {
        (buf[i], buf[len - i - 1]) = (buf[len - i - 1], buf[i]);
    }
    len
}

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
        let mut buf = [0u8; 64];
        let len = i32_to_str(fib(35), &mut buf);
        buf[len] = b'\n';
        core::arch::asm!(
            "int 0x80",
            in("rax") 1, // sys_write
            in("rdi") 0, // stderr
            in("rsi") buf.as_ptr(),
            in("rcx") len + 1,
        );
    }
    loop {}
}
