#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(str_from_raw_parts)]
#![allow(non_snake_case)]
#![allow(clippy::result_unit_err)]

extern crate alloc;
pub mod acpi;
pub mod apic;
pub mod blockdev;
pub mod console;
pub mod cpu;
pub mod int;
pub mod mm;
pub mod pcie;
pub mod rtc;
pub mod sound;
pub mod task;
pub mod vfs;
