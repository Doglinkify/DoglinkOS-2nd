#![feature(abi_x86_interrupt)]

use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::structures::idt::HandlerFunc;
use x86_64::structures::idt::InterruptStackFrame;
use crate::println;

pub static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

pub fn init() {
    unsafe {
        IDT.load();
    }
    register(32, handler);
}

pub fn register(n: u8, handler: HandlerFunc) {
    unsafe {
        IDT[n].set_handler_fn(handler);
    }
}

pub extern "x86-interrupt" fn handler(_: InterruptStackFrame) {
    println!("interrupt");
}
