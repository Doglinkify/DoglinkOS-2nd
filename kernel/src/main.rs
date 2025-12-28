#![no_std]
#![no_main]

use core::arch::asm;
use limine::request::{RequestsEndMarker, RequestsStartMarker};
use limine::BaseRevision;
use DoglinkOS_2nd::acpi::parse_madt;
use DoglinkOS_2nd::apic::{io::init as init_ioapic, local::init as init_lapic};
use DoglinkOS_2nd::blockdev::ahci::init as init_ahci;
use DoglinkOS_2nd::blockdev::nvme::init as init_nvme;
use DoglinkOS_2nd::console::init as init_terminal;
use DoglinkOS_2nd::cpu::show_cpu_info;
use DoglinkOS_2nd::int::init as init_interrupt;
use DoglinkOS_2nd::mm::init as init_mm;
use DoglinkOS_2nd::mm::page_alloc::test as test_page_alloc;
use DoglinkOS_2nd::pcie::enumrate::doit;
use DoglinkOS_2nd::println;
use DoglinkOS_2nd::task::{init as init_task, init_sse, reset_gdt};
use DoglinkOS_2nd::vfs::init as init_vfs;

#[used]
#[link_section = ".requests"]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(2);

/// Define the stand and end markers for Limine requests.
#[used]
#[link_section = ".requests_start_marker"]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[link_section = ".requests_end_marker"]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

#[no_mangle]
#[allow(named_asm_labels)]
#[allow(clippy::empty_loop)]
extern "C" fn kmain() -> ! {
    assert!(BASE_REVISION.is_supported());
    init_mm();
    init_terminal();
    println!(
        r"  ____                   _   _           _       ___    ____            ____                _
|  _ \    ___     __ _  | | (_)  _ __   | | __  / _ \  / ___|          |___ \   _ __     __| |
| | | |  / _ \   / _` | | | | | | '_ \  | |/ / | | | | \___ \   _____    __) | | '_ \   / _` |
| |_| | | (_) | | (_| | | | | | | | | | |   <  | |_| |  ___) | |_____|  / __/  | | | | | (_| |
|____/   \___/   \__, | |_| |_| |_| |_| |_|\_\  \___/  |____/          |_____| |_| |_|  \__,_|
                 |___/"
    );
    reset_gdt();
    init_interrupt();
    init_lapic();
    init_ioapic(parse_madt());
    init_ahci();
    init_nvme();
    show_cpu_info();
    show_pcie_info();
    test_page_alloc();
    init_vfs();
    init_sse();
    init_task();
    println!("[INFO] kmain: all things ok, let's start!");
    let fork_result: u64;
    unsafe {
        asm!(
            "int 0x80",
            in("rax") 2, // sys_fork
            out("rcx") fork_result,
        );
        if fork_result == 0 {
            asm!(
                "int 0x80",
                in("rax") 3, // sys_exec
                in("rdi") "/sbin/doglinked".as_ptr(),
                in("rcx") "/sbin/doglinked".len(),
            );
            unreachable!();
        } else {
            loop {}
        }
    }
}

fn show_pcie_info() {
    doit(|bus, device, function, config| {
        let vendor_id = config.vendor_id;
        let device_id = config.device_id;
        println!(
            "[INFO] kmain: found PCIe device: {:02x}:{:02x}.{} {:02x}{:02x}: {:04x}:{:04x}",
            bus, device, function, config.class_code, config.subclass, vendor_id, device_id
        );
    });
    let mut cnt = 0;
    doit(|_, _, _, _| cnt += 1);
    println!("[INFO] kmain: total {cnt} PCIe devices");
}

#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {:#?}", info);
    hang();
}

#[allow(clippy::empty_loop)]
fn hang() -> ! {
    loop {}
}
