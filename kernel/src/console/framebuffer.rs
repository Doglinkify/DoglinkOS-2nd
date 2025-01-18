use limine::framebuffer::Framebuffer as LimineFrameBuffer;
use spin::Mutex;

struct FrameBuffer {
    width: u64,
    height: u64,
    addr: u64,
    pitch: u64,
}

impl FrameBuffer {
    pub const fn null() -> FrameBuffer {
        FrameBuffer {
            width: 0,
            height: 0,
            addr: 0,
            pitch: 0,
        }
    }

    pub fn from_limine(fb: &LimineFrameBuffer) -> FrameBuffer {
        FrameBuffer {
            width: fb.width(),
            height: fb.height(),
            addr: fb.addr() as u64,
            pitch: fb.pitch(),
        }
    }
}

pub static display: Mutex<FrameBuffer> = Mutex::new(FrameBuffer::null());

pub fn init(fb: &LimineFrameBuffer) {
    *display.lock() = FrameBuffer::from_limine(fb);
}

pub fn display_fill(r: u8, g: u8, b: u8) {
    let lock = display.lock();
    for i in 0..(*lock).height {
        for j in 0..(*lock).width {
            unsafe {
                *(((*lock).addr as * mut u8).offset((i * (*lock).pitch + j * 4) as isize) as *mut u32) =
                    ((r as u32) << 16) + ((g as u32) << 8) + (b as u32);
            }
        }
    }
}

pub fn display_setpixel(x: u64, y: u64, r: u8, g: u8, b: u8) {
    let lock = display.lock();
    unsafe {
        *(((*lock).addr as * mut u8).offset((x * (*lock).pitch + y * 4) as isize) as *mut u32) =
            ((r as u32) << 16) + ((g as u32) << 8) + (b as u32);
    }
}
