#![no_std]
#![no_main]

#[panic_handler]
fn rust_panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 4, // sys_exit
        );
        unreachable!();
    }
}
