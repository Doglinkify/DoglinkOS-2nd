mod devfs;
mod fat;
mod procfs;

pub use fat::get_fs as get_fat_fs;

use crate::blockdev::ramdisk::RamDisk;
use crate::cmdline;
use crate::println;
use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use limine::module::InternalModule;
use limine::request::ModulesRequest;
use spin::{Lazy, Mutex};

#[used]
#[link_section = ".requests"]
pub(crate) static MODULE_REQUEST: ModulesRequest =
    ModulesRequest::new_rev1(&[&InternalModule::new(c"/initrd.img", c"initrd", 0)]);

pub fn has_cmdline_flag(flag: &str) -> bool {
    cmdline::has_cmdline_flag(flag)
}

static MOUNT_TABLE: Lazy<Vec<(String, Arc<dyn VfsDirectory + 'static>)>> = Lazy::new(Vec::new);

pub trait VfsDirectory: Send + Sync {
    fn file(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()>;
    fn directory(&self, path: &str) -> Result<Arc<Mutex<dyn VfsDirHandle + '_>>, ()>;
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

pub const DIRENT_NAME_CAP: usize = 255;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct DirEntry {
    pub is_dir: u8,
    pub name: [u8; DIRENT_NAME_CAP],
}

impl DirEntry {
    pub const fn empty() -> Self {
        Self {
            is_dir: 0,
            name: [0; DIRENT_NAME_CAP],
        }
    }

    pub fn new(is_dir: bool, name: &str) -> Self {
        let mut entry = Self::empty();
        entry.is_dir = is_dir as u8;
        let name_bytes = name.as_bytes();
        let copy_len = name_bytes.len().min(DIRENT_NAME_CAP - 1);
        entry.name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
        entry
    }
}

pub trait VfsDirHandle: Send {
    fn getdents(&mut self, buf: &mut [DirEntry]) -> usize;
}

pub struct SnapshotDirectory {
    entries: Vec<DirEntry>,
    offset: usize,
}

impl SnapshotDirectory {
    pub fn new(entries: Vec<DirEntry>) -> Self {
        Self { entries, offset: 0 }
    }
}

impl VfsDirHandle for SnapshotDirectory {
    fn getdents(&mut self, buf: &mut [DirEntry]) -> usize {
        let mut written = 0;
        while written < buf.len() && self.offset < self.entries.len() {
            buf[written] = self.entries[self.offset];
            self.offset += 1;
            written += 1;
        }
        written
    }
}

pub enum SeekFrom {
    Start(usize),
    End(isize),
    Current(isize),
}

pub fn init() {
    let file = MODULE_REQUEST.response().unwrap().modules()[0];
    let data = file.data();
    println!(
        "[DEBUG] vfs: initrd@{:?} has size {}",
        data.as_ptr(),
        data.len()
    );
    let disk = RamDisk::with_addr_and_size(data.as_ptr() as *mut u8, data.len() as u64);
    Lazy::force(&MOUNT_TABLE);
    mount(Some(disk), "/", self::fat::get_fs);
    mount(None::<RamDisk>, "/dev/", self::devfs::get_fs);
    mount(None::<RamDisk>, "/proc/", self::procfs::get_fs);
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

pub fn get_directory(path: &str) -> Result<Arc<Mutex<dyn VfsDirHandle>>, ()> {
    for fs in MOUNT_TABLE.iter() {
        if path.starts_with(&fs.0) {
            if let Ok(res) = fs.1.directory(&path[(fs.0.len() - 1)..]) {
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
        if path.starts_with(&fs.0) && fs.1.remove(&path[(fs.0.len() - 1)..]) {
            break;
        }
    }
}
