use alloc::sync::Arc;
use spin::Mutex;

use crate::sound::pcspk;
use crate::vfs::{SeekFrom, VfsFile};

struct PcspkDevice;

impl VfsFile for PcspkDevice {
    fn size(&mut self) -> usize {
        0
    }

    fn read(&mut self, _buf: &mut [u8]) -> usize {
        0
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        if let Ok(s) = core::str::from_utf8(buf) {
            if s.trim() == "stop" {
                unsafe { pcspk::stop_sound() }
            } else if let Ok(freq) = s.trim().parse() {
                unsafe { pcspk::play_sound(freq) }
            }
            buf.len()
        } else {
            0
        }
    }

    fn seek(&mut self, _pos: SeekFrom) -> usize {
        0
    }
}

pub(super) fn open(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    if path == "/pcspk" {
        Ok(Arc::new(Mutex::new(PcspkDevice)))
    } else {
        Err(())
    }
}
