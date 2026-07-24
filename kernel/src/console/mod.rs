mod framebuffer;
pub mod serial;

use alloc::boxed::Box;
use core::fmt::Write;
use core::sync::atomic::AtomicBool;
use crossbeam_queue::ArrayQueue;
use framebuffer::{FrameBuffer, FRAMEBUFFER_REQUEST};
use os_terminal::{font::BitmapFont, Terminal};
use spin::{Lazy, Mutex};

pub static FRAMEBUFFER: Lazy<FrameBuffer> = Lazy::new(|| {
    let framebuffer_response = FRAMEBUFFER_REQUEST.response().unwrap();
    let framebuffer = framebuffer_response.framebuffers()[0];
    FrameBuffer::from_limine(framebuffer)
});

pub static TERMINAL: Lazy<Mutex<Terminal<FrameBuffer>>> = Lazy::new(|| {
    let mut terminal = Terminal::new(*FRAMEBUFFER, Box::new(BitmapFont));
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
    serial::init();
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

#[macro_export]
macro_rules! dbg {
    () => {
        $crate::println!("[DEBUG] [{}:{}:{}]", file!(), line!(), column!());
    };
}

pub fn _print(args: core::fmt::Arguments) {
    TERMINAL.lock().write_fmt(args).unwrap();
    if serial::SERIAL_OK.load(core::sync::atomic::Ordering::Relaxed) {
        serial::Serial.write_fmt(args).unwrap();
    }
}

pub fn write(buf: &[u8]) {
    TERMINAL.lock().process(buf);
    if serial::SERIAL_OK.load(core::sync::atomic::Ordering::Relaxed) {
        serial::write_bytes(buf);
    }
}
