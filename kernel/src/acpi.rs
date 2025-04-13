use crate::mm::phys_to_virt;
use crate::println;
use limine::request::RsdpRequest;
use spin::Lazy;
use acpi::AcpiTables;
use acpi::handler::AcpiHandler;
use acpi::handler::PhysicalMapping;
use acpi::platform::interrupt::InterruptModel;
use acpi::PciConfigRegions;
use core::ptr::NonNull;

#[used]
#[link_section = ".requests"]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[derive(Copy, Clone)]
struct Handler;

impl AcpiHandler for Handler {
    unsafe fn map_physical_region<T>(&self, physical_address: usize, size: usize) -> PhysicalMapping<Self, T> {
        let va = {
            let virtual_address = crate::mm::phys_to_virt(physical_address as u64);
            NonNull::new_unchecked(virtual_address as *mut T)
        };
        PhysicalMapping::new(physical_address, va, size, size, *self)
    }

    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {}
}

pub static RSDP_PA: Lazy<usize> = Lazy::new(|| RSDP_REQUEST.get_response().unwrap().address() as usize);

pub fn parse_madt() -> u64 {
    println!("[INFO] acpi: parse_madt() called");
    let acpi = unsafe { AcpiTables::from_rsdp(Handler, *RSDP_PA - (phys_to_virt(0) as usize)).unwrap() };
    let res = acpi.platform_info().unwrap().interrupt_model;
    if let InterruptModel::Apic(apic) = res {
        let ioapic = apic.io_apics[0].address;
        ioapic as u64
    } else {
        panic!("acpi: unknown interrupt model");
    }
}

pub fn parse_mcfg() -> u64 {
    let acpi = unsafe { AcpiTables::from_rsdp(Handler, *RSDP_PA - (phys_to_virt(0) as usize)).unwrap() };
    let res = PciConfigRegions::new(&acpi).unwrap();
    let res2 = res.iter().next().unwrap().physical_address;
    res2 as u64
}
