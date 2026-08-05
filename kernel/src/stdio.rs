use core::sync::atomic::{AtomicU8, Ordering};

use crate::console::{self, ECHO_FLAG, serial};

#[repr(u8)]
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum Mode {
    None = 0,
    Serial = 1,
    Tty = 2,
    SerialTty = 3,
}

static MODE: AtomicU8 = AtomicU8::new(Mode::SerialTty as u8);

fn parse_mode(value: &str) -> Option<Mode> {
    match value {
        "none" => Some(Mode::None),
        "serial" => Some(Mode::Serial),
        "tty" => Some(Mode::Tty),
        "serial+tty" | "tty+serial" => Some(Mode::SerialTty),
        _ => None,
    }
}

pub fn init() {
    let mode = crate::cmdline::CMDLINE
        .split_ascii_whitespace()
        .find_map(|arg| arg.strip_prefix("stdio=").and_then(parse_mode))
        .unwrap_or(Mode::SerialTty);
    MODE.store(mode as u8, Ordering::Relaxed);
}

pub fn mode() -> Mode {
    match MODE.load(Ordering::Relaxed) {
        0 => Mode::None,
        1 => Mode::Serial,
        2 => Mode::Tty,
        _ => Mode::SerialTty,
    }
}

pub fn serial_enabled() -> bool {
    matches!(mode(), Mode::Serial | Mode::SerialTty)
}

pub fn tty_enabled() -> bool {
    matches!(mode(), Mode::Tty | Mode::SerialTty)
}

pub fn write_stdout(buf: &[u8]) {
    if tty_enabled() {
        console::TERMINAL.lock().process(buf);
    }
    if serial_enabled() && serial::SERIAL_OK.load(Ordering::Relaxed) {
        serial::write_bytes(buf);
    }
}

pub fn write_stderr(buf: &[u8]) {
    if tty_enabled() {
        let mut terminal = console::TERMINAL.lock();
        terminal.process(b"\x1b[31m");
        terminal.process(buf);
        terminal.process(b"\x1b[0m");
    }
    if serial_enabled() && serial::SERIAL_OK.load(Ordering::Relaxed) {
        serial::write_bytes(buf);
    }
}

pub fn read_stdin() -> Option<u8> {
    match mode() {
        Mode::None => None,
        Mode::Serial => read_serial(),
        Mode::Tty => console::INPUT_BUFFER.pop(),
        Mode::SerialTty => console::INPUT_BUFFER.pop().or_else(read_serial),
    }
}

fn read_serial() -> Option<u8> {
    if !serial::SERIAL_OK.load(Ordering::Relaxed) {
        return None;
    }
    let mut res = serial::read();
    if let Some(mut b) = res {
        if b == b'\r' {
            b = b'\n';
            res = Some(b);
        }
        if ECHO_FLAG.load(Ordering::Relaxed) {
            serial::write(b);
            if tty_enabled() {
                console::TERMINAL.lock().process(&[b]);
            }
        }
    }
    res
}
