#![no_std]
#![no_main]

use core::arch::asm;
use DoglinkOS_2nd::console::{init as init_console, clear as clear_console, console_setchar};
use limine::request::{FramebufferRequest, RequestsEndMarker, RequestsStartMarker};
use limine::BaseRevision;

#[used]
#[link_section = ".requests"]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[link_section = ".requests"]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

/// Define the stand and end markers for Limine requests.
#[used]
#[link_section = ".requests_start_marker"]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[link_section = ".requests_end_marker"]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

#[no_mangle]
extern "C" fn kmain() -> ! {
    // All limine requests must also be referenced in a called function, otherwise they may be
    // removed by the linker.
    assert!(BASE_REVISION.is_supported());

    if let Some(framebuffer_response) = FRAMEBUFFER_REQUEST.get_response() {
        if let Some(framebuffer) = framebuffer_response.framebuffers().next() {
            init_console(&framebuffer);
        }
    }
    clear_console();
    console_setchar(0, 0, 'H');
    console_setchar(0, 1, 'e');
    console_setchar(0, 2, 'l');
    console_setchar(0, 3, 'l');
    console_setchar(0, 4, 'o');
    console_setchar(0, 5, ',');
    console_setchar(0, 6, ' ');
    console_setchar(0, 7, 'W');
    console_setchar(0, 8, 'o');
    console_setchar(0, 9, 'r');
    console_setchar(0, 10, 'l');
    console_setchar(0, 11, 'd');
    console_setchar(0, 12, '!');
    hang();
}

#[panic_handler]
fn rust_panic(_info: &core::panic::PanicInfo) -> ! {
    hang();
}

fn hang() -> ! {
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}
