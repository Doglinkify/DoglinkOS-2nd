#![no_std]
#![no_main]

use dlos_app_rt::*;
use good_memory_allocator::SpinLockedAllocator;
use zune_jpeg::{JpegDecoder, zune_core::bytestream::ZCursor};

#[global_allocator]
static ALLOCATOR: SpinLockedAllocator = SpinLockedAllocator::empty();

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    init_heap();
    main();
    sys_exit();
}

fn init_heap() {
    unsafe {
        let old_brk: usize;
        core::arch::asm!(
            "int 0x80",
            in("rax") 7,
            in("rdi") 0,
            out("rsi") old_brk,
        );
        core::arch::asm!(
            "int 0x80",
            in("rax") 7,
            in("rdi") old_brk + (1 << 23),
            out("rsi") _,
        );
        ALLOCATOR.init(old_brk, 1 << 23);
    }
}

fn main() {
    let mut decoder = JpegDecoder::new(ZCursor::new(include_bytes!("test.jpg")));
    decoder.decode_headers().unwrap();
    println!("image is {:?}", decoder.dimensions().unwrap());
}
