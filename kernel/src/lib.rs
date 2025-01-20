#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(slice_take)]

pub mod acpi;
pub mod apic;
pub mod console;
pub mod cpu;
pub mod int;
pub mod mm;
pub mod pcie;
