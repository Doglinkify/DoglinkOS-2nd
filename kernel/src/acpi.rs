use crate::mm::phys_to_virt;
use crate::println;
use limine::request::RsdpRequest;
use spin::Mutex;

#[used]
#[link_section = ".requests"]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

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
    pointers: [u64; 32], // TODO
}

#[derive(Debug, Copy, Clone)]
#[repr(packed)]
pub struct MADT {
    header: ACPI_table_header,
    local_apic_addr: u32,
    flags: u32,
    var_marker: [u8; 1024], // to clone the rest of the MADT
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
    pointers: [0; 32],
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
    var_marker: [0; 1024],
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

pub unsafe fn init() {
    println!("[INFO] acpi: init() called");
    let res = RSDP_REQUEST.get_response().unwrap();
    let mut rsdp_lock = rsdp.lock();
    let mut xsdt_lock = xsdt.lock();
    *rsdp_lock = *(res.address() as *const RSDP);
    *xsdt_lock = *(phys_to_virt((*rsdp_lock).xsdt_addr) as *const XSDT);
    let xsdt_length = xsdt_lock.header.length;
    println!("[DEBUG] acpi: xsdt length {}", xsdt_length);
    for i in 0..32 {
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
    println!("[INFO] acpi: it didn't crash!");
}

pub fn parse_madt() -> u64 {
    println!("[INFO] acpi: parse_madt() called");
    let mut res: u64 = 0;
    let madt_lock = madt.lock();
    let madt_length = madt_lock.header.length;
    println!("[DEBUG] acpi: madt length {}", madt_length);
    let mut range = &((*madt_lock).var_marker)[..((*madt_lock).header.length as usize - 44)];
    // println!("{p:?} {edge:?}");
    while range != &[] {
        let entry_type = range[0];
        // println!("Entry type {}: {}", entry_type, ["Processor Local APIC", "I/O APIC",
        //                                            "I/O APIC Interrupt Source Override",
        //                                            "I/O APIC Non-maskable interrupt source",
        //                                            "Local APIC Non-maskable interrupts",
        //                                            "Local APIC Address Override",
        //                                            "Processor Local x2APIC"
        //                                           ][entry_type as usize]);
        // println!("[DEBUG] acpi: parse_madt(): zzjrabbit");
        // any println here will cause the real machine to reboot :(
        if entry_type == 1 {
            res = (range[4] as u64) + ((range[5] as u64) << 8) +
                ((range[6] as u64) << 16) + ((range[7] as u64) << 24);
            println!(
                "[DEBUG] DoglinkOS_2nd::acpi::parse_madt() will return {:?}",
                res as *const ()
            );
        }
        let entry_length = range[1];
        if entry_length > 14 {
            panic!("Abnormal entry length {entry_length}");
        }
        range.take(..(entry_length as usize)).unwrap();
    }
    if res == 0 {
        println!("[WRANING] acpi: parse_madt() will return 0!");
    }
    res
}
