use bit_field::BitField;
use core::slice;
use vcell::VolatileCell as Volatile;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{
    Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, Size4KiB,
};
use x86_64::VirtAddr;

use super::cmd::{CommandHeader, CommandTable, FisRegH2D};
use super::driver::Ahci;
use crate::mm::{page_alloc::alloc_physical_page, phys_to_virt};

const BLOCK_SIZE: usize = 512;
const SATA_SIG_ATAPI: u32 = 0xEB140101;
const SATA_SIG_SEMB: u32 = 0xC33C0101;
const SATA_SIG_PM: u32 = 0x96690101;

#[repr(C)]
pub struct HbaMemory {
    pub capability: Volatile<u32>,
    pub global_host_control: Volatile<u32>,
    pub interrupt_status: Volatile<u32>,
    pub port_implemented: Volatile<u32>,
    pub version: Volatile<u32>,
    pub ccc_control: Volatile<u32>,
    pub ccc_ports: Volatile<u32>,
    pub em_location: Volatile<u32>,
    pub em_control: Volatile<u32>,
    pub capabilities2: Volatile<u32>,
    pub bios_os_handoff_control: Volatile<u32>,
}

impl HbaMemory {
    pub fn enable_ahci(&self) {
        self.global_host_control
            .set(self.global_host_control.get() | (1 << 31));
    }

    pub fn disable_interrupt(&self) {
        self.global_host_control
            .set(self.global_host_control.get() & !(1 << 1));
    }

    pub fn port_active(&self, port_num: usize) -> bool {
        self.port_implemented.get().get_bit(port_num)
    }

    pub fn support_port_count(&self) -> usize {
        self.capability.get().get_bits(0..5) as usize + 1
    }

    pub fn get_port(&self, port_num: usize) -> Option<&HbaPort> {
        let hba_ptr = self as *const _ as usize;
        let port_address = hba_ptr + 0x100 + 0x80 * port_num;

        let port = unsafe { &*(port_address as *const HbaPort) };
        (port.device_connected() && port.is_sata_device()).then_some(port)
    }
}

#[repr(C)]
pub struct HbaPort {
    pub command_list_base_address: Volatile<u64>,
    pub fis_base_address: Volatile<u64>,
    pub interrupt_status: Volatile<u32>,
    pub interrupt_enable: Volatile<u32>,
    pub command: Volatile<u32>,
    pub reserved: Volatile<u32>,
    pub task_file_data: Volatile<u32>,
    pub signature: Volatile<u32>,
    pub sata_status: Volatile<u32>,
    pub sata_control: Volatile<u32>,
    pub sata_error: Volatile<u32>,
    pub sata_active: Volatile<u32>,
    pub command_issue: Volatile<u32>,
    pub sata_notification: Volatile<u32>,
    pub fis_based_switch_control: Volatile<u32>,
}

impl HbaPort {
    pub fn start_cmd(&self) {
        let command = &self.command;
        while command.get().get_bit(15) {}
        command.set(*command.get().set_bit(4, true));
        command.set(*command.get().set_bit(0, true));
    }

    pub fn stop_cmd_and_reset(&self) {
        let command = &self.command;
        command.set(*command.get().set_bit(0, false));
        command.set(*command.get().set_bit(4, false));
        while command.get().get_bit(15) || command.get().get_bit(14) {}
        let sata_control = &self.sata_control;
        sata_control.set((sata_control.get() & !0xf) | 1);
        for _ in 0..1000000 {
            unsafe {
                core::arch::asm!("pause");
            }
        }
        sata_control.set(sata_control.get() & !0xf);
    }

    pub fn is_sata_device(&self) -> bool {
        !matches!(
            self.signature.get(),
            SATA_SIG_ATAPI | SATA_SIG_SEMB | SATA_SIG_PM
        )
    }

    pub fn device_connected(&self) -> bool {
        let status = self.sata_status.get();
        status.get_bits(8..12) == 1 && status.get_bits(0..4) == 3
    }
}

impl HbaPort {
    pub(super) unsafe fn init_ahci(&'static self) -> Ahci {
        self.stop_cmd_and_reset();

        let cmd_list_pa = alloc_physical_page().unwrap();
        let cmd_list_va = phys_to_virt(cmd_list_pa);
        let cmd_table_pa = alloc_physical_page().unwrap();
        let cmd_table_va = phys_to_virt(cmd_table_pa);
        let data_pa = alloc_physical_page().unwrap();
        let data_va = phys_to_virt(data_pa);
        let fis_pa = alloc_physical_page().unwrap();
        let fis_va = phys_to_virt(fis_pa);

        let mut pgt = OffsetPageTable::new(
            &mut *(phys_to_virt(Cr3::read().0.start_address().as_u64()) as *mut PageTable),
            VirtAddr::new(crate::mm::phys_to_virt(0)),
        );
        let _ = pgt
            .update_flags(
                Page::<Size4KiB>::containing_address(VirtAddr::new(cmd_list_va)),
                PageTableFlags::WRITABLE | PageTableFlags::PRESENT | PageTableFlags::NO_CACHE,
            )
            .map(|u| u.flush());
        let _ = pgt
            .update_flags(
                Page::<Size4KiB>::containing_address(VirtAddr::new(cmd_table_va)),
                PageTableFlags::WRITABLE | PageTableFlags::PRESENT | PageTableFlags::NO_CACHE,
            )
            .map(|u| u.flush());
        let _ = pgt
            .update_flags(
                Page::<Size4KiB>::containing_address(VirtAddr::new(fis_va)),
                PageTableFlags::WRITABLE | PageTableFlags::PRESENT | PageTableFlags::NO_CACHE,
            )
            .map(|u| u.flush());
        let _ = pgt
            .update_flags(
                Page::<Size4KiB>::containing_address(VirtAddr::new(data_va)),
                PageTableFlags::WRITABLE | PageTableFlags::PRESENT | PageTableFlags::NO_CACHE,
            )
            .map(|u| u.flush());

        self.command_list_base_address.set(cmd_list_pa);
        self.fis_base_address.set(fis_pa);

        self.command_issue.set(0);

        let cmd_list_ptr = cmd_list_va as *mut CommandHeader;
        let cmd_list_size = 4096 / size_of::<CommandHeader>();
        let cmd_list = unsafe { slice::from_raw_parts_mut(cmd_list_ptr, cmd_list_size) };

        let cmd_header = &mut cmd_list[0];
        *cmd_header = core::mem::zeroed();
        cmd_header.command_table_base_address = cmd_table_pa;
        cmd_header.flags = (size_of::<FisRegH2D>() / size_of::<u32>()) as u16;
        cmd_header.prdt_length = 1;

        let cmd_table = &mut *(cmd_table_va as *mut CommandTable);
        *cmd_table = core::mem::zeroed();
        let prdt = &mut cmd_table.prdt[0];
        prdt.data_base_address = data_pa;
        prdt.byte_count_i = (BLOCK_SIZE - 1) as u32;

        let data = unsafe { slice::from_raw_parts_mut(data_va as *mut _, BLOCK_SIZE) };
        data.fill(0);

        self.start_cmd();

        Ahci {
            cmd_list,
            cmd_table,
            data,
            port: self,
            recieved_fis: unsafe { slice::from_raw_parts_mut(fis_va as *mut u8, 0x100) },
        }
    }
}
