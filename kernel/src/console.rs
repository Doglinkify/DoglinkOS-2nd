use limine::framebuffer::Framebuffer as LimineFrameBuffer;

struct FrameBuffer {
    width: u64,
    height: u64,
    addr: * mut u32,
    pitch: u64,
}

impl FrameBuffer {
    pub const fn null() -> FrameBuffer {
        FrameBuffer {
            width: 0,
            height: 0,
            addr: 0 as * mut u32,
            pitch: 0,
        }
    }

    pub fn from_limine(fb: &LimineFrameBuffer) -> FrameBuffer {
        FrameBuffer {
            width: fb.width(),
            height: fb.height(),
            addr: fb.addr() as * mut u32,
            pitch: fb.pitch(),
        }
    }

    pub fn fill(&self, r: u8, g: u8, b: u8) {
        unsafe {
            for i in 0..self.height {
                for j in 0..self.width {
                    *(self.addr.offset((i * self.pitch / 4 + j) as isize)) = 0xffffffffu32 /*(r as u32) << 16 + (g as u32) << 8 + (b as u32)*/;
                }
            }
        }
    }
}

static mut display: FrameBuffer = FrameBuffer::null();

pub fn init(fb: &LimineFrameBuffer) {
    unsafe {
        display = FrameBuffer::from_limine(fb);
    }
}

pub fn display_fill(r: u8, g: u8, b: u8) {
    unsafe {
        display.fill(r, g, b);
    }
}
