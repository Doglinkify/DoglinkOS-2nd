#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(array_ptr_get)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(non_upper_case_globals)]
#![allow(static_mut_refs)]

extern crate alloc;
pub mod acpi;
pub mod apic;
pub mod blockdev;
pub mod console;
pub mod cpu;
pub mod int;
pub mod mm;
pub mod pcie;
pub mod task;
pub mod vfs;
pub mod rtc;	
