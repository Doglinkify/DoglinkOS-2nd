#![no_std]
#![no_main]

extern crate alloc;

use dlos_app_rt::*;
use good_memory_allocator::SpinLockedAllocator;
use zune_jpeg::{JpegDecoder, zune_core::bytestream::ZCursor};
use zune_png::{PngDecoder, zune_core::result::DecodingResult};
use zune_ppm::PPMDecoder;
use zune_qoi::QoiDecoder;

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

fn read_file(path: &str) -> Option<alloc::vec::Vec<u8>> {
    if let Some(fd) = sys_open(path, false) {
        let size = sys_seek(fd, 0, 1);
        sys_seek(fd, 0, 2);
        let mut buf = alloc::vec![0u8; size];
        sys_read2(fd, &mut buf);
        sys_close(fd);
        Some(buf)
    } else {
        None
    }
}

fn read_line(buf: &mut [u8]) -> usize {
    for (i, v) in buf.iter_mut().enumerate() {
        match dlos_app_rt::sys_read() {
            b'\n' => return i,
            c => *v = c,
        }
    }
    buf.len()
}

fn process_file(path: &[u8]) -> (alloc::vec::Vec<u8>, usize, usize) {
    let path = core::str::from_utf8(path).unwrap();
    let file_content = read_file(path).unwrap();
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        let mut decoder = JpegDecoder::new(ZCursor::new(&file_content));
        decoder.decode_headers().unwrap();
        let (width, height) = decoder.dimensions().unwrap();
        let buf = decoder.decode().unwrap();
        (buf, width, height)
    } else if path.ends_with(".png") {
        let mut decoder = PngDecoder::new(ZCursor::new(&file_content));
        decoder.decode_headers().unwrap();
        let (width, height) = decoder.dimensions().unwrap();
        let res = decoder.decode().unwrap();
        match res {
            DecodingResult::U8(data) => (data, width, height),
            DecodingResult::U16(data) => {
                (data.iter().map(|x| (x >> 8) as u8).collect(), width, height)
            }
            _ => unreachable!(),
        }
    } else if path.ends_with(".ppm") {
        let mut decoder = PPMDecoder::new(ZCursor::new(&file_content));
        decoder.decode_headers().unwrap();
        let (width, height) = decoder.dimensions().unwrap();
        let res = decoder.decode().unwrap();
        match res {
            DecodingResult::U8(data) => (data, width, height),
            DecodingResult::U16(data) => {
                (data.iter().map(|x| (x >> 8) as u8).collect(), width, height)
            }
            _ => unreachable!(),
        }
    } else if path.ends_with(".qoi") {
        let mut decoder = QoiDecoder::new(ZCursor::new(&file_content));
        decoder.decode_headers().unwrap();
        let (width, height) = decoder.dimensions().unwrap();
        let res = decoder.decode().unwrap();
        (res, width, height)
    } else {
        (alloc::vec::Vec::new(), 0, 0)
    }
}

fn main() {
    print!("Image file path: ");
    let mut path_buf = [0; 128];
    let len = read_line(&mut path_buf);
    let path = &path_buf[0..len];
    let (buf, width, height) = process_file(path);
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
