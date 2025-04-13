mod framebuffer;

use spin::{Lazy, Mutex};
use os_terminal::{Terminal, font::BitmapFont};
use framebuffer::{FrameBuffer, FRAMEBUFFER_REQUEST};
use alloc::boxed::Box;
use core::fmt::Write;
use crossbeam_queue::ArrayQueue;

pub static TERMINAL: Lazy<Mutex<Terminal<FrameBuffer>>> = Lazy::new(|| {
    let framebuffer_response = FRAMEBUFFER_REQUEST.get_response().unwrap();
    let framebuffer = framebuffer_response.framebuffers().next().unwrap();
    let mut terminal = Terminal::new(FrameBuffer::from_limine(&framebuffer));
    terminal.set_font_manager(Box::new(BitmapFont));
    terminal.set_history_size(200);
    Mutex::new(terminal)
});

pub static INPUT_BUFFER: Lazy<ArrayQueue<u8>> = Lazy::new(|| ArrayQueue::new(128));

pub fn init() {
    Lazy::force(&TERMINAL);
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
    TERMINAL.lock().write_fmt(args).unwrap();
}
