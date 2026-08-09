//! Memory-only xHCI building blocks.
//!
//! This module deliberately has no controller discovery or startup side
//! effects yet.  The following commits build the hardware-facing lifecycle
//! on top of these checked layouts and ring primitives.

pub mod regs;
pub mod ring;
pub mod trb;

/// Run the xHCI pure-logic self tests.
pub fn test() {
    trb::test();
    ring::test();
    crate::println!("[INFO] xhci: TRB and ring self-tests passed");
}
