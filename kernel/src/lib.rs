#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(slice_take)]
#![feature(array_ptr_get)]
#![feature(naked_functions)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(non_upper_case_globals)]

extern crate alloc;
pub mod acpi;
pub mod apic;
pub mod console;
pub mod cpu;
pub mod int;
pub mod mm;
pub mod pcie;
pub mod blockdev;
pub mod task;
pub mod vfs;
