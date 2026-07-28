use alloc::borrow::ToOwned;
use alloc::string::String;
use limine::request::ExecutableCmdlineRequest;
use spin::Lazy;

#[used]
#[link_section = ".requests"]
static CMDLINE_REQUEST: ExecutableCmdlineRequest = ExecutableCmdlineRequest::new();

pub static CMDLINE: Lazy<String> = Lazy::new(|| {
    CMDLINE_REQUEST
        .response()
        .map(|resp| resp.cmdline().to_owned())
        .unwrap_or_default()
});

pub fn has_cmdline_flag(flag: &str) -> bool {
    CMDLINE.split_ascii_whitespace().any(|arg| arg == flag)
}
