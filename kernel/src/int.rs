#![feature(abi_x86_interrupt)]

use crate::{print, println};
use x86_64::structures::idt::HandlerFunc;
use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::structures::idt::InterruptStackFrame;
use x86_64::structures::idt::PageFaultErrorCode;
use spin::{Lazy, Mutex};

pub static IDT: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut temp = InterruptDescriptorTable::new();
    temp[32].set_handler_fn(handler1);
    temp[33].set_handler_fn(handler2);
    temp[34].set_handler_fn(handler3);
    temp[36].set_handler_fn(handler4);
    temp.page_fault.set_handler_fn(handler5);
    temp
});

pub fn init() {
    println!("[INFO] interrupt: init() called");
    IDT.load();
    println!("[INFO] interrupt: it didn't crash!");
    x86_64::instructions::interrupts::enable();
}

pub extern "x86-interrupt" fn handler1(_: InterruptStackFrame) {
//    print!(".");
    crate::apic::local::eoi();
}

pub extern "x86-interrupt" fn handler2(_: InterruptStackFrame) {
    println!("error interrupt");
}

pub extern "x86-interrupt" fn handler3(_: InterruptStackFrame) {
    println!("spurious interrupt");
}

pub extern "x86-interrupt" fn handler4(_: InterruptStackFrame) {
    print!("A");
    unsafe {
        let _: u8 = x86_64::instructions::port::PortReadOnly::new(0x60).read();
    }
    crate::apic::local::eoi();
}

pub extern "x86-interrupt" fn handler5(_: InterruptStackFrame, _1: PageFaultErrorCode) {
    println!("page fault");
    loop{}
}
