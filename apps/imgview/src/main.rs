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

fn get_framebuffer() -> (*mut u8, usize, usize, usize) {
    (
        sys_info(8).unwrap() as *mut u8,
        sys_info(6).unwrap(),
        sys_info(7).unwrap(),
        sys_info(9).unwrap(),
    )
}

fn main() {
    let mut decoder = JpegDecoder::new(ZCursor::new(include_bytes!("test.jpg")));
    decoder.decode_headers().unwrap();
    let (width, height) = decoder.dimensions().unwrap();
    let buf = decoder.decode().unwrap();
    let (ptr, fb_width, fb_height, pitch) = get_framebuffer();
    for i in 0..core::cmp::min(height, fb_height) {
        for j in 0..core::cmp::min(width, fb_width) {
            let base = (i * width + j) * 3;
            unsafe {
                *(ptr.add(i * pitch + j * 4) as *mut u32) = ((buf[base] as u32) << 16)
                    + ((buf[base + 1] as u32) << 8)
                    + (buf[base + 2] as u32)
            }
        }
    }
    _ = sys_read();
}
