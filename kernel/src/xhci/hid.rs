//! HID Boot keyboard and mouse report decoding.

use crate::inputdev::{self, MouseButtonKind, MouseEvent};

#[derive(Clone, Copy)]
pub struct KeyboardState {
    pressed: [u8; 32],
}

impl KeyboardState {
    pub const fn new() -> Self {
        Self { pressed: [0; 32] }
    }

    pub fn submit_report(&mut self, report: &[u8]) {
        self.for_each_scancode(report, inputdev::submit_keyboard_scancode);
    }

    fn for_each_scancode<F: FnMut(u8)>(&mut self, report: &[u8], mut submit: F) {
        if report.len() < 8 || report[2..8].contains(&1) {
            return;
        }
        let mut next = [0u8; 32];
        for modifier in 0..8 {
            if report[0] & (1 << modifier) != 0 {
                mark(&mut next, 0xe0 + modifier);
            }
        }
        for &usage in &report[2..8] {
            if usage != 0 {
                mark(&mut next, usage);
            }
        }
        for usage in 0..=u8::MAX {
            if marked(&self.pressed, usage)
                && !marked(&next, usage)
                && let Some(scancode) = set1_usage(usage)
            {
                submit(scancode | 0x80);
            }
        }
        for usage in 0..=u8::MAX {
            if !marked(&self.pressed, usage)
                && marked(&next, usage)
                && let Some(scancode) = set1_usage(usage)
            {
                submit(scancode);
            }
        }
        self.pressed = next;
    }
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MouseState {
    buttons: u8,
}

impl MouseState {
    pub const fn new() -> Self {
        Self { buttons: 0 }
    }

    pub fn submit_report(&mut self, report: &[u8]) {
        self.for_each_event(report, inputdev::submit_mouse_event);
    }

    fn for_each_event<F: FnMut(MouseEvent)>(&mut self, report: &[u8], mut submit: F) {
        if report.len() < 3 {
            return;
        }
        let buttons = report[0] & 0x07;
        for (bit, button) in [
            (0, MouseButtonKind::Left),
            (1, MouseButtonKind::Right),
            (2, MouseButtonKind::Middle),
        ] {
            if (buttons ^ self.buttons) & (1 << bit) != 0 {
                submit(MouseEvent::Button {
                    button,
                    pressed: buttons & (1 << bit) != 0,
                });
            }
        }
        self.buttons = buttons;
        let x = report[1] as i8 as isize;
        let y = report[2] as i8 as isize;
        if x != 0 || y != 0 {
            submit(MouseEvent::Move { x, y: -y });
        }
        if let Some(&wheel) = report.get(3) {
            let lines = wheel as i8 as isize;
            if lines != 0 {
                submit(MouseEvent::Scroll(lines));
            }
        }
    }
}

impl Default for MouseState {
    fn default() -> Self {
        Self::new()
    }
}

fn mark(bitmap: &mut [u8; 32], usage: u8) {
    bitmap[(usage / 8) as usize] |= 1 << (usage % 8);
}

fn marked(bitmap: &[u8; 32], usage: u8) -> bool {
    bitmap[(usage / 8) as usize] & (1 << (usage % 8)) != 0
}

/// HID Usage Tables keyboard page to the Set-1 codes consumed by the TTY.
fn set1_usage(usage: u8) -> Option<u8> {
    Some(match usage {
        0x04 => 0x1e,
        0x05 => 0x30,
        0x06 => 0x2e,
        0x07 => 0x20,
        0x08 => 0x12,
        0x09 => 0x21,
        0x0a => 0x22,
        0x0b => 0x23,
        0x0c => 0x17,
        0x0d => 0x24,
        0x0e => 0x25,
        0x0f => 0x26,
        0x10 => 0x32,
        0x11 => 0x31,
        0x12 => 0x18,
        0x13 => 0x19,
        0x14 => 0x10,
        0x15 => 0x13,
        0x16 => 0x1f,
        0x17 => 0x14,
        0x18 => 0x16,
        0x19 => 0x2f,
        0x1a => 0x11,
        0x1b => 0x2d,
        0x1c => 0x15,
        0x1d => 0x2c,
        0x1e => 0x02,
        0x1f => 0x03,
        0x20 => 0x04,
        0x21 => 0x05,
        0x22 => 0x06,
        0x23 => 0x07,
        0x24 => 0x08,
        0x25 => 0x09,
        0x26 => 0x0a,
        0x27 => 0x0b,
        0x28 => 0x1c,
        0x29 => 0x01,
        0x2a => 0x0e,
        0x2b => 0x0f,
        0x2c => 0x39,
        0x2d => 0x0c,
        0x2e => 0x0d,
        0x2f => 0x1a,
        0x30 => 0x1b,
        0x31 => 0x2b,
        0x33 => 0x27,
        0x34 => 0x28,
        0x35 => 0x29,
        0x36 => 0x33,
        0x37 => 0x34,
        0x38 => 0x35,
        0x39 => 0x3a,
        0xe0 | 0xe4 => 0x1d,
        0xe1 | 0xe5 => 0x2a,
        0xe2 | 0xe6 => 0x38,
        _ => return None,
    })
}

pub fn test() {
    assert_eq!(set1_usage(0x04), Some(0x1e));
    assert_eq!(set1_usage(0x28), Some(0x1c));
    assert_eq!(set1_usage(0xe1), Some(0x2a));
    assert_eq!(set1_usage(0x65), None);
    let mut keyboard = KeyboardState::new();
    let mut scancodes = [0u8; 4];
    let mut count = 0;
    keyboard.for_each_scancode(&[0, 0, 0x04, 0, 0, 0, 0, 0], |code| {
        scancodes[count] = code;
        count += 1;
    });
    keyboard.for_each_scancode(&[0, 0, 0, 0, 0, 0, 0, 0], |code| {
        scancodes[count] = code;
        count += 1;
    });
    assert_eq!(&scancodes[..count], &[0x1e, 0x9e]);

    let mut mouse = MouseState::new();
    let mut events = [None; 3];
    let mut count = 0;
    mouse.for_each_event(&[1, 2, 0xfe, 1], |event| {
        events[count] = Some(event);
        count += 1;
    });
    assert_eq!(
        &events[..count],
        &[
            Some(MouseEvent::Button {
                button: MouseButtonKind::Left,
                pressed: true,
            }),
            Some(MouseEvent::Move { x: 2, y: 2 }),
            Some(MouseEvent::Scroll(1)),
        ]
    );
}
