use alloc::vec::Vec;
use spin::Mutex;
use bit_field::BitField;
use crate::pcie::enumrate::doit;
use crate::println;
use crate::mm::phys_to_virt;

pub struct Disk;

#[repr(C)]
pub struct AhciHbaPort {
    command_list_base: u64,
    fis_base: u64,
    interrupt_status: u32,
    interrupt_enable: u32,
    command_and_status: u32,
    _rsv0: u32,
    task_file_data: u32,
    signature: u32,
    sata_status: u32,
    sata_control: u32,
    sata_error: u32,
    sata_active: u32,
    command_issue: u32,
    sata_ntf: u32,
    fis_based_switch_control: u32,
    _rsv1: [u32; 11],
    _vendor: [u32; 4],
}

#[repr(C)]
pub struct AhciHba {
    host_capability: u32,
    global_host_control: u32,
    interrupt_status: u32,
    port_implemented: u32,
    version: u32,
    ccc_control: u32,
    ccc_ports: u32,
    em_location: u32,
    em_control: u32,
    host_capability_2: u32,
    bohc: u32,
    _reserved: [u8; 212],
    ports: [AhciHbaPort; 32],
}

pub static AHCI: Mutex<Vec<Disk>> = Mutex::new(Vec::new());

pub fn get_disks(address: u64) -> Vec<Disk> {
    let hba = unsafe { &*(address as *const AhciHba) };
    for i in 0..32 {
        if hba.port_implemented.get_bit(i) {
            let port = &hba.ports[i];
            let ssts = port.sata_status;
            let ipm = (ssts >> 8) & 0x0f;
            let det = ssts & 0x0f;
            if det != 3 || ipm != 1 {
                //println!("[INFO] ahci: no drive found at port {}", i);
            } else {
                match port.signature {
                    0x00000101 => println!("[INFO] ahci: SATA drive found at port {}", i),
                    0xeb140101 => println!("[INFO] ahci: SATAPI drive found at port {}", i),
                    0xc33c0101 => println!("[INFO] ahci: SEMB drive found at port {}", i),
                    0x96690101 => println!("[INFO] ahci: PM found at port {}", i),
                    _ => println!("[WARN] ahci: ??? ({:08x}) drive found at port {}", port.signature, i),
                }
            }
        }
    }
    Vec::new()
}

pub fn init() {
    let mut ahci = AHCI.lock();
    doit(|_, _, _, config| {
        if config.class_code == 0x01 && config.subclass == 0x06 {
            let bar = config.bar[5];
            let address = bar & 0xfffffff0;
            ahci.extend(get_disks(phys_to_virt(address as u64)));
        }
    });
}
