use x2apic::lapic::{TimerMode, TimerDivide, LocalApic, LocalApicBuilder, xapic_base};
use crate::mm::phys_to_virt;
use crate::print;

static mut LAPIC: Option<LocalApic> = None;

pub fn init() {
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
        LAPIC = Some(lapic);
    }
}

pub fn eoi() {
    unsafe {
        LAPIC.as_mut().unwrap().end_of_interrupt();
    }
}
