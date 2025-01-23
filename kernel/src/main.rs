#![no_std]
#![no_main]

use core::arch::asm;
use limine::request::{
    FramebufferRequest, HhdmRequest, RequestsEndMarker, RequestsStartMarker, RsdpRequest,
};
use limine::BaseRevision;
use DoglinkOS_2nd::console::init as init_terminal;
use DoglinkOS_2nd::acpi::{init as init_acpi, parse_madt};
use DoglinkOS_2nd::apic::{io::init as init_ioapic, local::init as init_lapic};
use DoglinkOS_2nd::cpu::show_cpu_info;
use DoglinkOS_2nd::int::init as init_interrupt;
use DoglinkOS_2nd::mm::init as init_mm;
use DoglinkOS_2nd::pcie::enumrate::doit;
use DoglinkOS_2nd::println;

#[used]
#[link_section = ".requests"]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[link_section = ".requests"]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[link_section = ".requests"]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

/// Define the stand and end markers for Limine requests.
#[used]
#[link_section = ".requests_start_marker"]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[link_section = ".requests_end_marker"]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

#[no_mangle]
extern "C" fn kmain() -> ! {
    assert!(BASE_REVISION.is_supported());
    let hhdm_response = HHDM_REQUEST.get_response().unwrap();
    init_mm(&hhdm_response);
    init_terminal();
    println!("[INFO] Loading DoglinkOS GNU/MicroFish...");
    init_interrupt();
    init_lapic();
    let rsdp_response = RSDP_REQUEST.get_response().unwrap();
    unsafe { init_acpi(&rsdp_response) };
    init_ioapic(parse_madt());
    show_cpu_info();
    doit();
    hang();
}

#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {:#?}", info);
    hang();
}

fn hang() -> ! {
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}
