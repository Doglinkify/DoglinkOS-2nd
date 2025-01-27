use crate::mm::phys_to_virt;
use crate::println;
use x2apic::ioapic::IoApic;
use spin::Mutex;

static IOAPIC: Mutex<Option<IoApic>> = Mutex::new(None);

pub fn init(ioapic_phys_addr: u64) {
    println!("[INFO] ioapic: init() called");
    let ioapic_virt_addr = phys_to_virt(ioapic_phys_addr);
    *IOAPIC.lock() = unsafe {
        let mut tmp = IoApic::new(ioapic_virt_addr);
        tmp.init(35);
        tmp.enable_irq(1);
        let mut ent_1 = tmp.table_entry(1);
        ent_1.set_dest(0xff);
//        println!("{ent_1:#?}");
        tmp.set_table_entry(1, ent_1);
        Some(tmp)
    };
    println!("[INFO] it didn't crash!");
}
