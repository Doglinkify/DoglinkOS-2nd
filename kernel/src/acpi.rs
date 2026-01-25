use crate::mm::phys_to_virt;
use crate::println;
use acpi::fadt::Fadt;
use acpi::handler::AcpiHandler;
use acpi::handler::PhysicalMapping;
use acpi::mcfg::PciConfigEntry;
use acpi::platform::interrupt::InterruptModel;
use acpi::AcpiTables;
use acpi::PciConfigRegions;
use alloc::boxed::Box;
use alloc::vec::Vec;
use aml::AmlContext;
use core::ptr::NonNull;
use limine::request::RsdpRequest;
use spin::mutex::Mutex;
use spin::Lazy;
use x86_64::instructions::port::{PortReadOnly, PortWriteOnly};

#[used]
#[link_section = ".requests"]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[derive(Copy, Clone)]
struct Handler;

impl AcpiHandler for Handler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<Self, T> {
        let va = {
            let virtual_address = crate::mm::phys_to_virt(physical_address as u64);
            NonNull::new_unchecked(virtual_address as *mut T)
        };
        PhysicalMapping::new(physical_address, va, size, size, *self)
    }

    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {}
}

macro_rules! aml_impl {
    (mem read $typ:ident) => {
        paste::paste! {
            fn [<read_ $typ>](&self, address: usize) -> $typ {
                unsafe { core::ptr::read_volatile(phys_to_virt(address as u64) as *const $typ) }
            }
        }
    };
    (mem write $typ:ident) => {
        paste::paste! {
            fn [<write_ $typ>](&mut self, address: usize, value: $typ) {
                unsafe { core::ptr::write_volatile(phys_to_virt(address as u64) as *mut $typ, value) }
            }
        }
    };
    (io read $typ:ident) => {
        paste::paste! {
            fn [<read_io_ $typ>](&self, port: u16) -> $typ {
                unsafe { PortReadOnly::<$typ>::new(port).read() }
            }
        }
    };
    (io write $typ:ident) => {
        paste::paste! {
            fn [<write_io_ $typ>](&self, port: u16, value: $typ) {
                unsafe { PortWriteOnly::<$typ>::new(port).write(value) }
            }
        }
    };
}

impl aml::Handler for Handler {
    aml_impl!(mem read u8);
    aml_impl!(mem read u16);
    aml_impl!(mem read u32);
    aml_impl!(mem read u64);
    aml_impl!(mem write u8);
    aml_impl!(mem write u16);
    aml_impl!(mem write u32);
    aml_impl!(mem write u64);
    aml_impl!(io read u8);
    aml_impl!(io read u16);
    aml_impl!(io read u32);
    aml_impl!(io write u8);
    aml_impl!(io write u16);
    aml_impl!(io write u32);
    fn read_pci_u8(&self, _: u16, _: u8, _: u8, _: u8, _: u16) -> u8 {
        unimplemented!()
    }
    fn read_pci_u16(&self, _: u16, _: u8, _: u8, _: u8, _: u16) -> u16 {
        unimplemented!()
    }
    fn read_pci_u32(&self, _: u16, _: u8, _: u8, _: u8, _: u16) -> u32 {
        unimplemented!()
    }
    fn write_pci_u8(&self, _: u16, _: u8, _: u8, _: u8, _: u16, _: u8) {
        unimplemented!()
    }
    fn write_pci_u16(&self, _: u16, _: u8, _: u8, _: u8, _: u16, _: u16) {
        unimplemented!()
    }
    fn write_pci_u32(&self, _: u16, _: u8, _: u8, _: u8, _: u16, _: u32) {
        unimplemented!()
    }
}

pub static RSDP_PA: Lazy<usize> = Lazy::new(|| RSDP_REQUEST.get_response().unwrap().address());
pub static AML_CONTEXT: Lazy<Mutex<AmlContext>> = Lazy::new(|| {
    let acpi =
        unsafe { AcpiTables::from_rsdp(Handler, *RSDP_PA - (phys_to_virt(0) as usize)).unwrap() };
    let dsdt = acpi.dsdt().unwrap();
    let mut aml_context = AmlContext::new(Box::new(Handler), aml::DebugVerbosity::None);
    aml_context
        .parse_table(unsafe {
            core::slice::from_raw_parts(
                phys_to_virt(dsdt.address as u64) as *const u8,
                dsdt.length as usize,
            )
        })
        .unwrap();
    Mutex::new(aml_context)
});
pub static FADT: Lazy<Fadt> = Lazy::new(|| {
    let acpi =
        unsafe { AcpiTables::from_rsdp(Handler, *RSDP_PA - (phys_to_virt(0) as usize)).unwrap() };
    *acpi.find_table::<Fadt>().unwrap().get()
});

pub fn parse_madt() -> u64 {
    println!("[INFO] acpi: parse_madt() called");
    let acpi =
        unsafe { AcpiTables::from_rsdp(Handler, *RSDP_PA - (phys_to_virt(0) as usize)).unwrap() };
    let res = acpi.platform_info().unwrap().interrupt_model;
    if let InterruptModel::Apic(apic) = res {
        let ioapic = apic.io_apics[0].address;
        println!("[INFO] acpi: parse_madt() returned");
        ioapic as u64
    } else {
        panic!("acpi: unknown interrupt model");
    }
}

pub static PCI_CONFIG_REGIONS: Lazy<Vec<PciConfigEntry>> = Lazy::new(|| {
    let acpi =
        unsafe { AcpiTables::from_rsdp(Handler, *RSDP_PA - (phys_to_virt(0) as usize)).unwrap() };
    let res = PciConfigRegions::new(&acpi).unwrap();
    res.iter().collect()
});
