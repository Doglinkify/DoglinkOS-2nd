use limine::framebuffer::Framebuffer as LimineFrameBuffer;
use spin::Mutex;
use os_terminal::{DrawTarget, Rgb};
use limine::request::FramebufferRequest;

#[used]
#[link_section = ".requests"]
pub static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

pub struct FrameBuffer {
    width: usize,
    height: usize,
    addr: usize,
    pitch: usize,
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
            width: fb.width() as usize,
            height: fb.height() as usize,
            addr: fb.addr() as usize,
            pitch: fb.pitch() as usize,
        }
    }
}

impl DrawTarget for FrameBuffer {
    fn size(&self) -> (usize, usize) {
        (self.width as usize, self.height as usize)
    }

    #[inline(always)]
    fn draw_pixel(&mut self, x: usize, y: usize, color: Rgb) {
        unsafe {
            *((self.addr as * mut u8).offset((y * self.pitch + x * 4) as isize) as *mut u32) =
                ((color.0 as u32) << 16) + ((color.1 as u32) << 8) + (color.2 as u32);
        }
    }
}
