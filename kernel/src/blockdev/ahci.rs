use alloc::vec::Vec;
use spin::Mutex;
use bit_field::BitField;
use alloc::boxed::Box;
use vcell::VolatileCell as Volatile;
use crate::pcie::enumrate::doit;
use crate::println;
use crate::mm::phys_to_virt;

#[repr(C)]
pub struct AhciCommandHeader {
    flags: u16,
    prdt_length: u16,
    prd_byte_count: Volatile<u32>,
    command_table_base: u64,
    _reserved: [u32; 4],
}

#[repr(C)]
pub struct AhciCommandTable {
    cfis: [u8; 64],
    acmd: [u8; 16],
    _reserved: [u8; 48],
    prd_data_base: u64,
    _reserved2: u32,
    prd_byte_count: u32,
}

#[repr(C)]
pub struct AhciFisRegH2D {
    fis_type: u8,
    cflags: u8,
    command: u8,
    feature_lo: u8,
    lba_0: u8,
    lba_1: u8,
    lba_2: u8,
    device: u8,
    lba_3: u8,
    lba_4: u8,
    lba_5: u8,
    feature_hi: u8,
    sector_count: u16,
    icc: u8,
    control: u8,
    _padding: [u8; 4],
}

pub struct Disk {
    command_list_entry_0: &'static mut AhciCommandHeader,
    command_table: &'static mut AhciCommandTable,
    data: &'static mut [u8],
    port: &'static AhciHbaPort,
}

unsafe impl Send for Disk {}

impl Disk {
    pub fn identify(&mut self) {
        let fis = unsafe { &mut *(&mut self.command_table.cfis as *mut [u8; 64] as *mut AhciFisRegH2D) };
        {
            let port_base = self.port as *const _;
            println!("port base = {:?}", port_base);
            let cmdlist_base_1 = self.port.command_list_base.get();
            let cmdlist_base_2 = self.command_list_entry_0 as *const _;
            println!("cmdlist base = 0x{:08x} or {:?}", cmdlist_base_1, cmdlist_base_2);
            let cmdtab_base_1 = self.command_list_entry_0.command_table_base;
            let cmdtab_base_2 = self.command_table as *const _;
            println!("cmdtab base = 0x{:08x} or {:?}", cmdtab_base_1, cmdtab_base_2);
            let prd_data_base_1 = self.command_table.prd_data_base;
            let prd_data_base_2 = self.data as *const _;
            println!("prd data base = 0x{:08x} or {:?}", prd_data_base_1, prd_data_base_2);
        }
        fis.fis_type = 0x27;
        fis.cflags = 1 << 7;
        fis.command = 0xec;
        fis.device = 0;
        fis.sector_count = 0;
        fis.lba_0 = 0;
        fis.lba_1 = 0;
        fis.lba_2 = 0;
        fis.lba_3 = 0;
        fis.lba_4 = 0;
        fis.lba_5 = 0;
        self.port.command_issue.set(1);
        println!("[INFO] ahci: command issue sent");
        while self.port.command_issue.get().get_bit(0) {}
        println!("[INFO] ahci: disk identify returns {:?}", self.data);
    }
}

#[repr(C)]
pub struct AhciHbaPort {
    command_list_base: Volatile<u64>,
    fis_base: Volatile<u64>,
    interrupt_status: Volatile<u32>,
    interrupt_enable: Volatile<u32>,
    command_and_status: Volatile<u32>,
    _rsv0: Volatile<u32>,
    task_file_data: Volatile<u32>,
    signature: Volatile<u32>,
    sata_status: Volatile<u32>,
    sata_control: Volatile<u32>,
    sata_error: Volatile<u32>,
    sata_active: Volatile<u32>,
    command_issue: Volatile<u32>,
    sata_ntf: Volatile<u32>,
    fis_based_switch_control: Volatile<u32>,
    _rsv1: [u32; 11],
    _vendor: [u32; 4],
}

#[repr(C)]
pub struct AhciHba {
    host_capability: Volatile<u32>,
    global_host_control: Volatile<u32>,
    interrupt_status: Volatile<u32>,
    port_implemented: Volatile<u32>,
    version: Volatile<u32>,
    ccc_control: Volatile<u32>,
    ccc_ports: Volatile<u32>,
    em_location: Volatile<u32>,
    em_control: Volatile<u32>,
    host_capability_2: Volatile<u32>,
    bohc: Volatile<u32>,
    _reserved: [u8; 212],
    ports: [AhciHbaPort; 32],
}

pub static AHCI: Mutex<Vec<Disk>> = Mutex::new(Vec::new());

pub fn get_disk_from_port(port: &'static AhciHbaPort) -> Disk {
    let command_list_entry_0 = unsafe { &mut *(phys_to_virt(port.command_list_base.get()) as *mut AhciCommandHeader) };
    let data_pa = crate::mm::page_alloc::alloc_physical_page().unwrap();
    let cmdtab_pa = data_pa + 512;
    let data_va = phys_to_virt(data_pa);
    let cmdtab_va = phys_to_virt(cmdtab_pa);
    let data = unsafe { &mut *(data_va as *mut [u8; 512]) };
    let command_table = unsafe { &mut *(cmdtab_va as *mut AhciCommandTable)};
    command_list_entry_0.command_table_base = cmdtab_pa;
    command_list_entry_0.flags = (size_of::<AhciFisRegH2D>() / size_of::<u32>()) as u16;
    command_list_entry_0.prdt_length = 1;
    command_table.prd_data_base = data_pa;
    command_table.prd_byte_count = 511;
    Disk {
        command_list_entry_0,
        command_table,
        data,
        port,
    }
}

impl Drop for Disk {
    fn drop(&mut self) {
        crate::mm::page_alloc::dealloc_physical_page(crate::mm::virt_to_phys(self.data.as_ptr() as u64));
    }
}

pub fn get_disks(address: u64) -> Vec<Disk> {
    let hba = unsafe { &*(address as *const AhciHba) };
    let mut res: Vec<Disk> = Vec::new();
    for i in 0..32 {
        if hba.port_implemented.get().get_bit(i) {
            let port = &hba.ports[i];
            let ssts = port.sata_status.get();
            let ipm = (ssts >> 8) & 0x0f;
            let det = ssts & 0x0f;
            if det != 3 || ipm != 1 {
                println!("[INFO] ahci: no drive found at port {}", i);
            } else {
                match port.signature.get() {
                    0x00000101 => {
                        println!("[INFO] ahci: SATA drive found at port {}", i);
                        res.push(get_disk_from_port(port));
                    },
                    0xeb140101 => println!("[INFO] ahci: SATAPI drive found at port {}, ignoring!", i),
                    0xc33c0101 => println!("[INFO] ahci: SEMB drive found at port {}, ignoring!", i),
                    0x96690101 => println!("[INFO] ahci: PM found at port {}, ignoring!", i),
                    _ => println!("[WARN] ahci: ??? ({:08x}) drive found at port {}, ignoring!", port.signature.get(), i),
                }
            }
        }
    }
    res
}

pub fn init() {
    println!("[DEBUG] ahci: struct AhciCommandHeader has size {}", core::mem::size_of::<AhciCommandHeader>());
    println!("[DEBUG] ahci: struct AhciCommandTable has size {}", core::mem::size_of::<AhciCommandTable>());
    println!("[DEBUG] ahci: struct AhciHbaPort has size {}", core::mem::size_of::<AhciHbaPort>());
    println!("[DEBUG] ahci: struct AhciHba has size {}", core::mem::size_of::<AhciHba>());
    let mut ahci = AHCI.lock();
    doit(|_, _, _, config| {
        if config.class_code == 0x01 && config.subclass == 0x06 {
            let bar = config.bar[5];
            let address = bar & 0xfffffff0;
            ahci.extend(get_disks(phys_to_virt(address as u64)));
        }
    });
    for disk in ahci.iter_mut() {
        disk.identify();
    }
}
