use crate::mm::phys_to_virt;
use x2apic::ioapic::IoApic;

static mut IOAPIC: Option<IoApic> = None;

pub fn init(ioapic_phys_addr: u64) {
    let ioapic_virt_addr = phys_to_virt(ioapic_phys_addr);
    unsafe {
        IOAPIC = Some(IoApic::new(ioapic_virt_addr));
        IOAPIC.as_mut().unwrap().init(35);
        IOAPIC.as_mut().unwrap().enable_irq(1);
    }
}
