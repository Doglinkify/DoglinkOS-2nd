use x2apic::lapic::{LocalApic, LocalApicBuilder, xapic_base};
use crate::mm::phys_to_virt;

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
    }
}
