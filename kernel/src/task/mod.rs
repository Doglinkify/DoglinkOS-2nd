pub mod process;

use core::arch::asm;
use spin::Lazy;
use x86_64::structures::gdt::{GlobalDescriptorTable, Descriptor};
use x86_64::registers::segmentation::{CS, DS, SS, ES, FS, GS, Segment, SegmentSelector};
use x86_64::PrivilegeLevel;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::frame::PhysFrame;
use x86_64::structures::tss::TaskStateSegment;
use x86_64::addr::PhysAddr;
use x86_64::addr::VirtAddr;

pub static TSS: Lazy<TaskStateSegment> = Lazy::new(|| {
    let mut tss = TaskStateSegment::new();
    let rsp0_pa = crate::mm::page_alloc::alloc_physical_page().unwrap();
    tss.privilege_stack_table[0] = VirtAddr::new(crate::mm::phys_to_virt(rsp0_pa));
    tss
});

pub static GDT: Lazy<GlobalDescriptorTable> = Lazy::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    gdt.append(Descriptor::kernel_code_segment());
    gdt.append(Descriptor::kernel_data_segment());
    gdt.append(Descriptor::user_code_segment());
    gdt.append(Descriptor::user_data_segment());
    gdt.append(Descriptor::tss_segment(&TSS));
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
        x86_64::instructions::tables::load_tss(SegmentSelector::new(5, PrivilegeLevel::Ring0));
    }
}

#[allow(named_asm_labels)]
pub fn init() {
    x86_64::instructions::interrupts::enable();
    unsafe {
        let flags = Cr3::read().1;
        let new_cr3_va;
        {
            let mut tasks = self::process::TASKS.lock();
            tasks[0] = Some(self::process::Process::new());
            new_cr3_va = tasks[0].as_ref().unwrap().page_table.level_4_table() as *const _ as u64;
        }
        let new_cr3 = PhysFrame::from_start_address(
            PhysAddr::new(
                new_cr3_va - crate::mm::phys_to_virt(0)
            )
        ).unwrap();
        crate::println!("[DEBUG] task: will load task 0's cr3 {:?}", new_cr3);
        Cr3::write(new_cr3, flags);
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
