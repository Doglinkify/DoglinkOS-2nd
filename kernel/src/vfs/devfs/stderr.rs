use alloc::sync::Arc;
use spin::Mutex;

use crate::vfs::{SeekFrom, VfsFile};

struct StderrDevice;

impl VfsFile for StderrDevice {
    fn size(&mut self) -> usize {
        0
    }

    fn read(&mut self, _buf: &mut [u8]) -> usize {
        0
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        crate::console::write(b"\x1b[31m");
        crate::console::write(buf);
        crate::console::write(b"\x1b[0m");
        buf.len()
    }

    fn seek(&mut self, _pos: SeekFrom) -> usize {
        0
    }
}

pub(super) fn open(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    if path == "/stderr" {
        Ok(Arc::new(Mutex::new(StderrDevice)))
    } else {
        Err(())
    }
}
