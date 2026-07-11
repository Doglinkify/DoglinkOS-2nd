use crate::mm::phys_to_virt;
use crate::println;
use spin::Mutex;
use x2apic::ioapic::IoApic;
#[cfg(not(feature = "ps2_poll"))]
use x2apic::ioapic::{IrqFlags, IrqMode};

static IOAPIC: Mutex<Option<IoApic>> = Mutex::new(None);

pub fn init(ioapic_phys_addr: u64, lapic_id: u8) {
    println!("[INFO] ioapic: init() called");
    let ioapic_virt_addr = phys_to_virt(ioapic_phys_addr);
    *IOAPIC.lock() = unsafe {
        let mut tmp = IoApic::new(ioapic_virt_addr);
        tmp.init(35);
        #[cfg(not(feature = "ps2_poll"))]
        {
            for irq in 0..=tmp.max_table_entry() {
                tmp.disable_irq(irq);
            }
            let mut ent_1 = tmp.table_entry(1);
            ent_1.set_vector(36);
            ent_1.set_mode(IrqMode::Fixed);
            ent_1.set_flags(IrqFlags::empty());
            ent_1.set_dest(lapic_id);
            tmp.set_table_entry(1, ent_1);
            tmp.enable_irq(1);
            let mut ent_12 = tmp.table_entry(12);
            ent_12.set_vector(47);
            ent_12.set_mode(IrqMode::Fixed);
            ent_12.set_flags(IrqFlags::empty());
            ent_12.set_dest(lapic_id);
            tmp.set_table_entry(12, ent_12);
            tmp.enable_irq(12);
        }
        #[cfg(feature = "ps2_poll")]
        {
            _ = lapic_id;
        }
        Some(tmp)
    };
    println!("[INFO] it didn't crash!");
}
