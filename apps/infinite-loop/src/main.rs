#![no_std]
#![no_main]

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    dlos_app_rt::sys_exit();
}
