use alloc::sync::Arc;
use spin::Mutex;

use crate::vfs::{SeekFrom, VfsFile, CMDLINE};

struct CmdlineDevice {
    pos: usize,
}

impl VfsFile for CmdlineDevice {
    fn size(&mut self) -> usize {
        CMDLINE.len()
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        let data = CMDLINE.as_bytes();
        if self.pos >= data.len() {
            return 0;
        }
        let len = buf.len().min(data.len() - self.pos);
        buf[..len].copy_from_slice(&data[self.pos..self.pos + len]);
        self.pos += len;
        len
    }

    fn write(&mut self, _buf: &[u8]) -> usize {
        0
    }

    fn seek(&mut self, pos: SeekFrom) -> usize {
        let len = CMDLINE.len();
        let new_pos = match pos {
            SeekFrom::Start(pos) => pos.min(len),
            SeekFrom::End(offset) => len.saturating_add_signed(offset).min(len),
            SeekFrom::Current(offset) => self.pos.saturating_add_signed(offset).min(len),
        };
        self.pos = new_pos;
        self.pos
    }
}

pub(super) fn open(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    if path == "/cmdline" {
        Ok(Arc::new(Mutex::new(CmdlineDevice { pos: 0 })))
    } else {
        Err(())
    }
}
