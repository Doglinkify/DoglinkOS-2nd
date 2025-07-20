mod framebuffer;

use alloc::boxed::Box;
use core::fmt::Write;
use core::sync::atomic::AtomicBool;
use crossbeam_queue::ArrayQueue;
use framebuffer::{FrameBuffer, FRAMEBUFFER_REQUEST};
use os_terminal::{font::BitmapFont, Terminal};
use spin::{Lazy, Mutex};

pub static TERMINAL: Lazy<Mutex<Terminal<FrameBuffer>>> = Lazy::new(|| {
    let framebuffer_response = FRAMEBUFFER_REQUEST.get_response().unwrap();
    let framebuffer = framebuffer_response.framebuffers().next().unwrap();
    let mut terminal = Terminal::new(FrameBuffer::from_limine(&framebuffer));
    terminal.set_font_manager(Box::new(BitmapFont));
    terminal.set_history_size(200);
    terminal.set_crnl_mapping(true);
    terminal.set_pty_writer(Box::new(|s| {
        for b in s.as_bytes() {
            ECHO_BUFFER.force_push(*b);
        }
    }));
    Mutex::new(terminal)
});

pub static ECHO_BUFFER: Lazy<ArrayQueue<u8>> = Lazy::new(|| ArrayQueue::new(128));

pub static INPUT_BUFFER: Lazy<ArrayQueue<u8>> = Lazy::new(|| ArrayQueue::new(128));

pub static ECHO_FLAG: AtomicBool = AtomicBool::new(true);

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
