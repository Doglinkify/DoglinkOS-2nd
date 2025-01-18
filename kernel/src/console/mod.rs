mod framebuffer;

use limine::framebuffer::Framebuffer as LimineFrameBuffer;
use noto_sans_mono_bitmap::{get_raster, FontWeight, RasterHeight};
use spin::Mutex;

struct Console {
    width: u64,
    height: u64,
    cursor_x: u64,
    cursor_y: u64,
}

impl Console {
    pub const fn null() -> Console {
        Console {
            width: 0,
            height: 0,
            cursor_x: 0,
            cursor_y: 0,
        }
    }

    pub fn clear(&mut self) {
        framebuffer::display_fill(0, 0, 0);
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    pub fn setchar(&mut self, x: u64, y: u64, c: char) {
        let rc = get_raster(c, FontWeight::Regular, RasterHeight::Size16).unwrap();
        for (i, di) in rc.raster().iter().enumerate() {
            for (j, dj) in di.iter().enumerate() {
                framebuffer::display_setpixel(x * 16 + i as u64, y * 7 + j as u64, *dj, *dj, *dj);
            }
        }
    }

    pub fn cr(&mut self) {
        self.cursor_y = 0;
    }

    pub fn newline(&mut self) {
        self.cr();
        self.cursor_x += 1;
        if self.cursor_x == self.width {
            self.cursor_x = 0;
            self.clear();
        }
    }

    pub fn inc(&mut self) {
        self.cursor_y += 1;
        if self.cursor_y == self.width {
            self.newline();
        }
    }

    pub fn putchar(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.cr(),
            oc => {
                self.setchar(self.cursor_x, self.cursor_y, oc);
                self.inc();
            }
        }
    }

    pub fn puts(&mut self, s: &str) {
        for c in s.chars() {
            self.putchar(c);
        }
    }
}

pub static console: Mutex<Console> = Mutex::new(Console::null());

pub fn init(fb: &LimineFrameBuffer) {
    let mut lock = console.lock();
    framebuffer::init(fb);
    (*lock).width = fb.width() / 7;
    (*lock).height = fb.height() / 16;
    (*lock).clear();
}

impl core::fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.puts(s);
        Ok(())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::console::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

pub fn _print(args: core::fmt::Arguments) {
    core::fmt::write(&mut *console.lock(), args);
}
