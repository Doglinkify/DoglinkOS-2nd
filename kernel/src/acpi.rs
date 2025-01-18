use crate::mm::phys_to_virt;
use crate::println;
use limine::response::RsdpResponse;
use spin::Mutex;

#[derive(Debug, Copy, Clone)]
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

#[derive(Debug, Copy, Clone)]
#[repr(packed)]
pub struct XSDT {
    header: ACPI_table_header,
    pointers: [u64; 16], // TODO
}

#[derive(Debug, Copy, Clone)]
#[repr(packed)]
pub struct MADT {
    header: ACPI_table_header,
    local_apic_addr: u32,
    flags: u32,
    var_marker: [u8; 128], // to clone the rest of the MADT
}

#[derive(Debug, Copy, Clone)]
#[repr(packed)]
pub struct PCIE_CFG_ALLOC {
    pub base_addr: u64,
    pub pci_segment_group_number: u16,
    pub start_pci_bus_number: u8,
    pub end_pci_bus_number: u8,
    pub reserved: u32,
}

#[derive(Debug, Copy, Clone)]
#[repr(packed)]
pub struct MCFG {
    header: ACPI_table_header,
    reserved: u64,
    pub alloc: PCIE_CFG_ALLOC,
}

pub static rsdp: Mutex<RSDP> = Mutex::new(RSDP {
    signature: [0; 8],
    checksum: 0,
    OEMID: [0; 6],
    revision: 0,
    rsdt_addr: 0,
    length: 0,
    xsdt_addr: 0,
    ext_checksum: 0,
    reserved: [0; 3],
});

pub static xsdt: Mutex<XSDT> = Mutex::new(XSDT {
    header: ACPI_table_header {
        signature: [0; 4],
        length: 0,
        revision: 0,
        checksum: 0,
        OEMID: [0; 6],
        OEMTableID: [0; 8],
        OEMRevision: 0,
        creator_id: 0,
        creator_revison: 0,
    },
    pointers: [0; 16],
});

pub static madt: Mutex<MADT> = Mutex::new(MADT {
    header: ACPI_table_header {
        signature: [0; 4],
        length: 0,
        revision: 0,
        checksum: 0,
        OEMID: [0; 6],
        OEMTableID: [0; 8],
        OEMRevision: 0,
        creator_id: 0,
        creator_revison: 0,
    },
    local_apic_addr: 0,
    flags: 0,
    var_marker: [0; 128],
});

pub static mcfg: Mutex<MCFG> = Mutex::new(MCFG {
    header: ACPI_table_header {
        signature: [0; 4],
        length: 0,
        revision: 0,
        checksum: 0,
        OEMID: [0; 6],
        OEMTableID: [0; 8],
        OEMRevision: 0,
        creator_id: 0,
        creator_revison: 0,
    },
    reserved: 0,
    alloc: PCIE_CFG_ALLOC {
        base_addr: 0,
        pci_segment_group_number: 0,
        start_pci_bus_number: 0,
        end_pci_bus_number: 0,
        reserved: 0,
    },
});

pub unsafe fn init(res: &RsdpResponse) {
    let mut rsdp_lock = rsdp.lock();
    let mut xsdt_lock = xsdt.lock();
    *rsdp_lock = *(res.address() as *const RSDP);
    *xsdt_lock = *(phys_to_virt((*rsdp_lock).xsdt_addr) as *const XSDT);
    for i in 0..16 {
        if (*xsdt_lock).pointers[i] == 0 {
            break;
        }
        let head = &*(phys_to_virt((*xsdt_lock).pointers[i]) as *const ACPI_table_header);
        if head.signature == [65, 80, 73, 67] {
            // "APIC"
            *madt.lock() = *(head as *const ACPI_table_header as *const MADT);
        } else if head.signature == [77, 67, 70, 71] {
            // "MCFG"
            *mcfg.lock() = *(head as *const ACPI_table_header as *const MCFG);
        }
    }
}

pub unsafe fn parse_madt() -> u64 {
    let mut res: u64 = 0;
    let madt_lock = madt.lock();
    let mut p = &((*madt_lock).var_marker) as *const u8;
    let edge = (&(*madt_lock) as * const MADT as *const u8).offset((*madt_lock).header.length as isize);
    // println!("{p:?} {edge:?}");
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
            let res_addr = p.offset(4) as *const u32;
            res = *res_addr as u64;
            println!(
                "DoglinkOS_2nd::acpi::parse_madt() will return {:?}",
                res as *const ()
            );
        }
        let entry_length = *(p.offset(1));
        p = p.offset((entry_length) as isize);
    }
    res
}
