use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

use crate::vfs::{DirEntry, VfsDirHandle, VfsDirectory, VfsFile};

pub(super) struct DevFileSystem;

struct DevDirectory {
    entries: Vec<DirEntry>,
    offset: usize,
}

impl VfsDirectory for DevFileSystem {
    fn file(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()> {
        if let Ok(file) = super::disk::open(path) {
            Ok(file)
        } else if let Ok(file) = super::nvme::open(path) {
            Ok(file)
        } else if let Ok(file) = super::initrd::open(path) {
            Ok(file)
        } else if let Ok(file) = super::stdout::open(path) {
            Ok(file)
        } else if let Ok(file) = super::stderr::open(path) {
            Ok(file)
        } else if let Ok(file) = super::tty::open(path) {
            Ok(file)
        } else if let Ok(file) = super::serial::open(path) {
            Ok(file)
        } else if let Ok(file) = super::cmdline::open(path) {
            Ok(file)
        } else if let Ok(file) = super::pcspk::open(path) {
            Ok(file)
        } else if let Ok(file) = super::power::open(path) {
            Ok(file)
        } else {
            Err(())
        }
    }

    fn directory(&self, path: &str) -> Result<Arc<Mutex<dyn VfsDirHandle + '_>>, ()> {
        if path != "/" && !path.is_empty() {
            return Err(());
        }

        let mut entries = vec![
            DirEntry::new(false, "initrd"),
            DirEntry::new(false, "stdout"),
            DirEntry::new(false, "stderr"),
            DirEntry::new(false, "tty"),
            DirEntry::new(false, "serial"),
            DirEntry::new(false, "cmdline"),
            DirEntry::new(false, "pcspk"),
            DirEntry::new(false, "power"),
        ];

        for (idx, _) in crate::blockdev::ahci::AHCI.iter().enumerate() {
            entries.push(DirEntry::new(false, &alloc::format!("disk{idx}")));
        }

        for (device_idx, device) in crate::blockdev::nvme::NVME.iter().enumerate() {
            for namespace_idx in 0..device.len() {
                entries.push(DirEntry::new(
                    false,
                    &alloc::format!("nvme{device_idx}-{namespace_idx}"),
                ));
            }
        }

        Ok(Arc::new(Mutex::new(DevDirectory { entries, offset: 0 })))
    }

    fn create_file_or_open_existing(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()> {
        self.file(path)
    }

    fn remove(&self, _path: &str) -> bool {
        false
    }
}

impl VfsDirHandle for DevDirectory {
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
