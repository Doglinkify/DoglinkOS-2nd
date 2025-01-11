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

#[derive(Debug)]
#[repr(packed)]
pub struct XSDT {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    OEMID: [u8; 6],
    OEMTableID: [u8; 8],
    OEMRevision: u32,
    creator_id: u32,
    creator_revison: u32,
    pointers: [* const (); 16], // TODO
}

pub static mut rsdp: * const RSDP = 0 as * const RSDP;
pub static mut xsdt: * const XSDT = 0 as * const XSDT;

pub fn init(res: &RsdpResponse) {
    unsafe {
        rsdp = res.address() as * const RSDP;
        println!("{:?}", *rsdp);
        xsdt = phys_to_virt((*rsdp).xsdt_addr) as * const XSDT;
        println!("{:?}", *xsdt);
    }
}
