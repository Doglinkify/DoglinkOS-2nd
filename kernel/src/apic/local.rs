use crate::mm::phys_to_virt;
use crate::println;
use x2apic::lapic::{xapic_base, LocalApic, LocalApicBuilder, TimerDivide, TimerMode};
use spin::Mutex;

static LAPIC: Mutex<Option<LocalApic>> = Mutex::new(None);

fn disable_pic() {
    unsafe {
        x86_64::instructions::port::PortWriteOnly::new(0x21).write(0xffu8);
        x86_64::instructions::port::PortWriteOnly::new(0xA1).write(0xffu8);
    }
}

pub fn init() {
    println!("[INFO] lapic: init() called");
    disable_pic(); // IMPORTANT
    let apic_phys_addr = unsafe { xapic_base() };
    let mut lapic = LocalApicBuilder::new()
        .timer_vector(32)
        .error_vector(33)
        .spurious_vector(34)
        .set_xapic_base(phys_to_virt(apic_phys_addr))
        .build()
        .unwrap();
    unsafe {
        lapic.enable();
        lapic.set_timer_mode(TimerMode::Periodic);
        lapic.set_timer_divide(TimerDivide::Div2);
        lapic.enable_timer();
        lapic.set_timer_initial(0xffffffu32);
        *LAPIC.lock() = Some(lapic);
    }
    println!("[INFO] lapic: it didn't crash!");
}

pub fn eoi() {
    unsafe {
       (*LAPIC.lock()).as_mut().unwrap().end_of_interrupt();
    }
}
