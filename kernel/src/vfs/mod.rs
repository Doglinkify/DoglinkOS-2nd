use limine::request::ModuleRequest;
use limine::modules::InternalModule;
use crate::blockdev::ramdisk::RamDisk;
use crate::println;
use fatfs::{FileSystem, FsOptions};

#[used]
#[link_section = ".requests"]
static MODULE_REQUEST: ModuleRequest = ModuleRequest::new().with_internal_modules(&[
    &InternalModule::new().with_path(limine::cstr!("/initrd.img"))
]);

static mut ROOTFS: Option<FileSystem<RamDisk>> = None;

pub fn init() {
    let file = MODULE_REQUEST.get_response().unwrap().modules()[0];
    println!("[DEBUG] vfs: initrd@{:?} has size {}", file.addr(), file.size());
    let disk = RamDisk::with_addr_and_size(file.addr(), file.size());
    unsafe {
        ROOTFS = Some(FileSystem::new(disk, FsOptions::new()).unwrap());
    }
}

#[allow(static_mut_refs)]
pub fn get_file(path: &str) -> fatfs::File<RamDisk, fatfs::DefaultTimeProvider, fatfs::LossyOemCpConverter> {
    unsafe {
        ROOTFS.as_ref().unwrap().root_dir().open_file(path).unwrap()
    }
}
