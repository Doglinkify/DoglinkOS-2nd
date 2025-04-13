use crate::println;
use core::arch::naked_asm;
use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::structures::idt::InterruptStackFrame;
use x86_64::structures::idt::PageFaultErrorCode;
use x86_64::registers::control::Cr2;
use x86_64::PrivilegeLevel;
use spin::Lazy;

pub static IDT: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut temp = InterruptDescriptorTable::new();
    temp[32].set_handler_fn(handler1);
    temp[33].set_handler_fn(handler2);
    temp[34].set_handler_fn(handler3);
    temp[36].set_handler_fn(handler4);
    temp[0x80].set_handler_fn(crate::task::syscall::syscall_handler).set_privilege_level(PrivilegeLevel::Ring3);
    temp.page_fault.set_handler_fn(handler5);
    temp.general_protection_fault.set_handler_fn(handler6);
    temp
});

pub fn init() {
    println!("[INFO] interrupt: init() called");
    IDT.load();
    println!("[INFO] interrupt: it didn't crash!");
}

#[naked]
pub extern "x86-interrupt" fn handler1(_: InterruptStackFrame) {
    unsafe {
        naked_asm!(
            "push r15",
            "push r14",
            "push r13",
            "push r12",
            "push r11",
            "push r10",
            "push r9",
            "push r8",
            "push rdi",
            "push rbp",
            "push rsi",
            "push rdx",
            "push rcx",
            "push rbx",
            "push rax",
            "mov rdi, rsp",
            "mov rsi, 0",
            "call {}",
            "pop rax",
            "pop rbx",
            "pop rcx",
            "pop rdx",
            "pop rsi",
            "pop rbp",
            "pop rdi",
            "pop r8",
            "pop r9",
            "pop r10",
            "pop r11",
            "pop r12",
            "pop r13",
            "pop r14",
            "pop r15",
            "iretq",
            sym crate::task::sched::schedule,
        )
    }
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

#[allow(clippy::empty_loop)]
pub extern "x86-interrupt" fn handler5(f: InterruptStackFrame, code: PageFaultErrorCode) {
    match f.code_segment.rpl() {
        PrivilegeLevel::Ring0 => {
            println!(
                "page fault in kernel code, caused by instruction at {:?}, addr: {:?}, code: {:?}",
                f.instruction_pointer, Cr2::read().unwrap(), code
            );
            loop {}
        },
        PrivilegeLevel::Ring3 => {
            crate::mm::page_alloc::do_user_page_fault(code)
        },
        _ => unreachable!() // Ring1 and Ring2 is unused in this kernel
    }
}

#[allow(clippy::empty_loop)]
pub extern "x86-interrupt" fn handler6(f: InterruptStackFrame, c: u64) {
    println!("general protection fault, caused by instruction at {:?}, code: {}", f.instruction_pointer, c);
    loop{}
}
