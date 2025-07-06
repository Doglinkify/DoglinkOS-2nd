mod fat;

use crate::blockdev::ramdisk::RamDisk;
use crate::println;
use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use limine::modules::InternalModule;
use limine::request::ModuleRequest;

#[used]
#[link_section = ".requests"]
static MODULE_REQUEST: ModuleRequest =
    ModuleRequest::new().with_internal_modules(&[&InternalModule::new().with_path(c"/initrd.img")]);

static mut MOUNT_TABLE: Option<Vec<(String, Box<dyn VfsDirectory>)>> = None;

pub trait VfsDirectory: Send {
    fn file(&self, path: &str) -> Result<Box<dyn VfsFile + '_>, ()>;
}

pub trait VfsFile {
    fn size(&mut self) -> usize;
    fn read(&mut self, buf: &mut [u8]) -> usize;
    fn write(&mut self, buf: &mut [u8]) -> usize;
    fn seek(&mut self, pos: SeekFrom) -> usize;
    fn read_exact(&mut self, buf: &mut [u8]) {
        let mut buf2 = buf;
        while !buf2.is_empty() {
            match self.read(buf2) {
                0 => break,
                n => buf2 = &mut buf2[n..],
            }
        }
    }
}

pub enum SeekFrom {
    Start(usize),
    End(isize),
    Current(isize),
}

pub fn init() {
    let file = MODULE_REQUEST.get_response().unwrap().modules()[0];
    println!(
        "[DEBUG] vfs: initrd@{:?} has size {}",
        file.addr(),
        file.size()
    );
    unsafe {
        MOUNT_TABLE = Some(Vec::new());
    }
    let disk = RamDisk::with_addr_and_size(file.addr(), file.size());
    mount(disk, "/", self::fat::get_fs);
}

pub fn mount<T>(device: T, path: &str, fs: fn(T) -> Box<dyn VfsDirectory>)
where
    T: fatfs::ReadWriteSeek,
{
    unsafe {
        MOUNT_TABLE
            .as_mut()
            .unwrap()
            .push((path.to_owned(), fs(device)));
    }
}

pub fn get_file(path: &str) -> Result<Box<dyn VfsFile>, ()> {
    unsafe {
        for fs in MOUNT_TABLE.as_mut().unwrap().iter() {
            if path.starts_with(&fs.0) {
                if let Ok(res) = fs.1.file(&path[(fs.0.len() - 1)..]) {
                    return Ok(res);
                }
            }
        }
    }
    Err(())
}
