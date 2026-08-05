mod framebuffer;
pub mod serial;

use alloc::boxed::Box;
use core::fmt::Write;
use core::sync::atomic::AtomicBool;
use crossbeam_queue::ArrayQueue;
use framebuffer::{FRAMEBUFFER_REQUEST, FrameBuffer};
use os_terminal::{Terminal, font::BitmapFont};
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
    crate::stdio::init();
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
    struct Stdout;
    impl Write for Stdout {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            crate::stdio::write_stdout(s.as_bytes());
            Ok(())
        }
    }
    let _ = Stdout.write_fmt(args);
}

pub fn write(buf: &[u8]) {
    crate::stdio::write_stdout(buf);
}

pub fn write_err(buf: &[u8]) {
    crate::stdio::write_stderr(buf);
}
