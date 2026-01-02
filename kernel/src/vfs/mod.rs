mod devfs;
mod fat;

use crate::blockdev::ramdisk::RamDisk;
use crate::println;
use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use limine::modules::InternalModule;
use limine::request::ModuleRequest;
use spin::{Lazy, Mutex};

#[used]
#[link_section = ".requests"]
static MODULE_REQUEST: ModuleRequest =
    ModuleRequest::new().with_internal_modules(&[&InternalModule::new().with_path(c"/initrd.img")]);

static MOUNT_TABLE: Lazy<Vec<(String, Arc<dyn VfsDirectory + 'static>)>> = Lazy::new(Vec::new);

pub trait VfsDirectory: Send + Sync {
    fn file(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()>;
    fn create_file_or_open_existing(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()>;
    fn remove(&self, path: &str) -> bool;
}

pub trait VfsFile: Send {
    fn size(&mut self) -> usize;
    fn read(&mut self, buf: &mut [u8]) -> usize;
    fn write(&mut self, buf: &[u8]) -> usize;
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
    fn write_all(&mut self, buf: &[u8]) {
        let mut buf2 = buf;
        while !buf2.is_empty() {
            match self.write(buf2) {
                0 => break,
                n => buf2 = &buf2[n..],
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
    let disk = RamDisk::with_addr_and_size(file.addr(), file.size());
    Lazy::force(&MOUNT_TABLE);
    mount(Some(disk), "/", self::fat::get_fs);
    mount(None::<RamDisk>, "/dev/", self::devfs::get_fs);
}

pub fn mount<T>(device: Option<T>, path: &str, fs: fn(Option<T>) -> Arc<dyn VfsDirectory>)
where
    T: fatfs::ReadWriteSeek,
{
    unsafe {
        (*Lazy::as_mut_ptr(&MOUNT_TABLE)).push((path.to_owned(), fs(device)));
    }
}

pub fn get_file(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    for fs in MOUNT_TABLE.iter() {
        if path.starts_with(&fs.0) {
            if let Ok(res) = fs.1.file(&path[(fs.0.len() - 1)..]) {
                return Ok(res);
            }
        }
    }
    Err(())
}

pub fn create_file_or_open_existing(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    for fs in MOUNT_TABLE.iter() {
        if path.starts_with(&fs.0) {
            if let Ok(res) = fs.1.create_file_or_open_existing(&path[(fs.0.len() - 1)..]) {
                return Ok(res);
            }
        }
    }
    Err(())
}

pub fn remove_file(path: &str) {
    for fs in MOUNT_TABLE.iter() {
        if path.starts_with(&fs.0)
            && fs.1.remove(&path[(fs.0.len() - 1)..]) {
                break;
            }
    }
}
