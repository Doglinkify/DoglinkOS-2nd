#![feature(abi_x86_interrupt)]

use crate::console::putchar;
use crate::println;
use x86_64::structures::idt::HandlerFunc;
use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::structures::idt::InterruptStackFrame;

pub static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

pub fn init() {
    unsafe {
        IDT.load();
    }
    register(32, handler1);
    register(33, handler2);
    register(34, handler3);
    register(36, handler4);
    x86_64::instructions::interrupts::enable();
}

pub fn register(n: u8, handler: HandlerFunc) {
    unsafe {
        IDT[n].set_handler_fn(handler);
    }
}

pub extern "x86-interrupt" fn handler1(_: InterruptStackFrame) {
    putchar('.');
    crate::apic::local::eoi();
}

pub extern "x86-interrupt" fn handler2(_: InterruptStackFrame) {
    println!("error interrupt");
}

pub extern "x86-interrupt" fn handler3(_: InterruptStackFrame) {
    println!("spurious interrupt");
}

pub extern "x86-interrupt" fn handler4(_: InterruptStackFrame) {
    putchar('A');
    unsafe {
        let _: u8 = x86_64::instructions::port::PortReadOnly::new(0x60).read();
    }
    crate::apic::local::eoi();
}
