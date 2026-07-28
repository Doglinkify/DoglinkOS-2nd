use alloc::sync::Arc;
use spin::Mutex;

use crate::vfs::{SeekFrom, VfsFile};

struct StdoutDevice;

impl VfsFile for StdoutDevice {
    fn size(&mut self) -> usize {
        0
    }

    fn read(&mut self, _buf: &mut [u8]) -> usize {
        0
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        crate::console::write(buf);
        buf.len()
    }

    fn seek(&mut self, _pos: SeekFrom) -> usize {
        0
    }
}

pub(super) fn open(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    if path == "/stdout" {
        Ok(Arc::new(Mutex::new(StdoutDevice)))
    } else {
        Err(())
    }
}
