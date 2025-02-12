#![feature(abi_x86_interrupt)]

use crate::{print, println};
use x86_64::structures::idt::HandlerFunc;
use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::structures::idt::InterruptStackFrame;
use x86_64::structures::idt::PageFaultErrorCode;
use x86_64::registers::control::Cr2;
use spin::{Lazy, Mutex};

pub static IDT: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut temp = InterruptDescriptorTable::new();
    temp[32].set_handler_fn(handler1);
    temp[33].set_handler_fn(handler2);
    temp[34].set_handler_fn(handler3);
    temp[36].set_handler_fn(handler4);
    temp.page_fault.set_handler_fn(handler5);
    temp.general_protection_fault.set_handler_fn(handler6);
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
    unsafe {
        let scancode: u8 = x86_64::instructions::port::PortReadOnly::new(0x60).read();
        let mut lock = crate::console::TERMINAL.lock();
        if let Some(s) = lock.handle_keyboard(scancode) {
            lock.process(s.as_bytes());
        }
    }
    crate::apic::local::eoi();
}

pub extern "x86-interrupt" fn handler5(f: InterruptStackFrame, code: PageFaultErrorCode) {
    println!("page fault, caused by instruction at {:?}, addr: {:?}, code: {:?}", f.instruction_pointer, Cr2::read().unwrap(), code);
    loop{}
}

pub extern "x86-interrupt" fn handler6(f: InterruptStackFrame, c: u64) {
    println!("general protection fault, caused by instruction at {:?}, code: {}", f.instruction_pointer, c);
    loop{}
}
