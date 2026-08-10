//! Memory-only xHCI building blocks.
//!
//! This module deliberately has no controller discovery or startup side
//! effects yet.  The following commits build the hardware-facing lifecycle
//! on top of these checked layouts and ring primitives.

mod controller;
pub mod hid;
pub mod regs;
pub mod ring;
pub mod trb;
pub mod usb;

pub use controller::{ControllerState, init, interrupt_handler, poll};

/// Run the xHCI pure-logic self tests.
pub fn test() {
    trb::test();
    ring::test();
    usb::test();
    hid::test();
    controller::test();
    crate::println!("[INFO] xhci: core self-tests passed");
}
