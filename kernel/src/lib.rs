#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(str_from_raw_parts)]
#![allow(non_snake_case)]
#![allow(clippy::result_unit_err)]
#![allow(clippy::len_without_is_empty)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::too_many_arguments)]

extern crate alloc;
pub mod acpi;
pub mod apic;
pub mod blockdev;
pub mod cmdline;
pub mod console;
pub mod cpu;
pub mod inputdev;
pub mod int;
pub mod mm;
pub mod pcie;
pub mod power;
pub mod rtc;
pub mod sound;
pub mod stdio;
pub mod task;
pub mod vfs;
pub mod xhci;
