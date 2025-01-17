use limine::response::RsdpResponse;
use crate::mm::phys_to_virt;
use crate::println;

#[derive(Debug)]
#[repr(packed)]
pub struct RSDP {
    signature: [u8; 8],
    checksum: u8,
    OEMID: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
    length: u32,
    xsdt_addr: u64,
    ext_checksum: u8,
    reserved: [u8; 3],
}

#[derive(Debug, Copy, Clone)]
#[repr(packed)]
pub struct ACPI_table_header {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    OEMID: [u8; 6],
    OEMTableID: [u8; 8],
    OEMRevision: u32,
    creator_id: u32,
    creator_revison: u32,
}

#[derive(Debug)]
#[repr(packed)]
pub struct XSDT {
    header: ACPI_table_header,
    pointers: [u64; 16], // TODO
}

#[derive(Debug)]
#[repr(packed)]
pub struct MADT {
    header: ACPI_table_header,
    local_apic_addr: u32,
    flags: u32,
}

pub static mut rsdp: * const RSDP = 0 as * const RSDP;
pub static mut xsdt: * const XSDT = 0 as * const XSDT;
pub static mut madt: * const MADT = 0 as * const MADT;

pub fn init(res: &RsdpResponse) {
    unsafe {
        rsdp = res.address() as * const RSDP;
        // println!("{:?}", *rsdp);
        xsdt = phys_to_virt((*rsdp).xsdt_addr) as * const XSDT;
        // println!("{:?}", *xsdt);
        for i in 0..16 {
            let head = phys_to_virt((*xsdt).pointers[i]) as * const ACPI_table_header;
            if (*head).signature == [65, 80, 73, 67] { // "APIC"
                madt = head as * const MADT;
                break;
                // println!("{:?}", *madt);
            }
        }
    }
}

pub fn parse_madt() -> u64 {
    let mut res: u64 = 0;
    unsafe {
        let mut p = madt.offset(1) as * const u8;
        let edge = (madt as * const u8).offset((*madt).header.length as isize);
        while p < edge {
            let entry_type = *p;
            // println!("Entry type {}: {}", entry_type, ["Processor Local APIC", "I/O APIC",
            //                                            "I/O APIC Interrupt Source Override",
            //                                            "I/O APIC Non-maskable interrupt source",
            //                                            "Local APIC Non-maskable interrupts",
            //                                            "Local APIC Address Override",
            //                                            "Processor Local x2APIC"
            //                                           ][entry_type as usize]);
            if entry_type == 1 {
                let res_addr = p.offset(4) as * const u32;
                res = *res_addr as u64;
                println!("DoglinkOS_2nd::acpi::parse_madt() will return {:?}", res as * const ());
            }
            let entry_length = *(p.offset(1));
            p = p.offset((entry_length) as isize);
        }
    }
    res
}
