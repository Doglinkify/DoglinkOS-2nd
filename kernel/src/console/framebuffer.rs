use limine::framebuffer::Framebuffer as LimineFrameBuffer;

struct FrameBuffer {
    width: u64,
    height: u64,
    addr: *mut u8,
    pitch: u64,
}

impl FrameBuffer {
    pub const fn null() -> FrameBuffer {
        FrameBuffer {
            width: 0,
            height: 0,
            addr: 0 as *mut u8,
            pitch: 0,
        }
    }

    pub fn from_limine(fb: &LimineFrameBuffer) -> FrameBuffer {
        FrameBuffer {
            width: fb.width(),
            height: fb.height(),
            addr: fb.addr(),
            pitch: fb.pitch(),
        }
    }
}

pub static mut display: FrameBuffer = FrameBuffer::null();

pub fn init(fb: &LimineFrameBuffer) {
    unsafe {
        display = FrameBuffer::from_limine(fb);
    }
}

pub fn display_fill(r: u8, g: u8, b: u8) {
    unsafe {
        for i in 0..display.height {
            for j in 0..display.width {
                *(display.addr.offset((i * display.pitch + j * 4) as isize) as *mut u32) =
                    ((r as u32) << 16) + ((g as u32) << 8) + (b as u32);
            }
        }
    }
}

pub fn display_setpixel(x: u64, y: u64, r: u8, g: u8, b: u8) {
    unsafe {
        *(display.addr.offset((x * display.pitch + y * 4) as isize) as *mut u32) =
            ((r as u32) << 16) + ((g as u32) << 8) + (b as u32);
    }
}
