use alloc::sync::Arc;
use spin::Mutex;

use crate::vfs::{VfsDirectory, VfsFile};

pub(super) struct DevFileSystem;

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

    fn create_file_or_open_existing(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()> {
        self.file(path)
    }

    fn remove(&self, _path: &str) -> bool {
        false
    }
}
