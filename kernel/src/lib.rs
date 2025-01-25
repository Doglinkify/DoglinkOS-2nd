#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(slice_take)]
#![feature(array_ptr_get)]

extern crate alloc;
pub mod acpi;
pub mod apic;
pub mod console;
pub mod cpu;
pub mod int;
pub mod mm;
pub mod pcie;
pub mod blockdev;
