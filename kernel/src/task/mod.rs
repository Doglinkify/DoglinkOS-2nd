pub mod process;

use core::arch::asm;
use spin::Lazy;
use x86_64::structures::gdt::{GlobalDescriptorTable, Descriptor};
use x86_64::registers::segmentation::{CS, DS, SS, ES, FS, GS, Segment, SegmentSelector};
use x86_64::PrivilegeLevel;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::frame::PhysFrame;
use x86_64::addr::PhysAddr;

pub static GDT: Lazy<GlobalDescriptorTable> = Lazy::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    gdt.append(Descriptor::kernel_code_segment());
    gdt.append(Descriptor::kernel_data_segment());
    gdt.append(Descriptor::user_code_segment());
    gdt.append(Descriptor::user_data_segment());
    gdt
});

pub fn reset_gdt() {
    GDT.load();
    unsafe {
        CS::set_reg(SegmentSelector::new(1, PrivilegeLevel::Ring0));
        DS::set_reg(SegmentSelector::new(2, PrivilegeLevel::Ring0));
        SS::set_reg(SegmentSelector::new(2, PrivilegeLevel::Ring0));
        ES::set_reg(SegmentSelector::new(2, PrivilegeLevel::Ring0));
        FS::set_reg(SegmentSelector::new(2, PrivilegeLevel::Ring0));
        GS::set_reg(SegmentSelector::new(2, PrivilegeLevel::Ring0));
    }
}

#[allow(named_asm_labels)]
pub fn init() {
    x86_64::instructions::interrupts::disable();
    unsafe {
        let flags = Cr3::read().1;
        crate::println!("Adsciocewhfrui");
        let addr = PhysFrame::from_start_address(
            PhysAddr::new(
                self::process::TASKS[0].page_table.as_ref().unwrap().level_4_table() as *const _ as u64 - crate::mm::phys_to_virt(0)
            )
        ).unwrap();
        crate::println!("will load cr3 with {:?} {:?}", addr, flags);
        Cr3::write(addr, flags);
        DS::set_reg(SegmentSelector::new(4, PrivilegeLevel::Ring3));
        ES::set_reg(SegmentSelector::new(4, PrivilegeLevel::Ring3));
        FS::set_reg(SegmentSelector::new(4, PrivilegeLevel::Ring3));
        GS::set_reg(SegmentSelector::new(4, PrivilegeLevel::Ring3));
        asm!(
            "mov rax, rsp",
            "push 0x23",
            "push rax",
            "pushfq",
            "push 0x1b",
            "push offset cd",
            "iretq",
            "cd:",
            out("rax") _,
        );
    }
}
