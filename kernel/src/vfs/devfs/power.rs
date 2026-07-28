use alloc::sync::Arc;
use spin::Mutex;

use crate::power;
use crate::vfs::{SeekFrom, VfsFile};

struct PowerDevice;

impl VfsFile for PowerDevice {
    fn size(&mut self) -> usize {
        0
    }

    fn read(&mut self, _buf: &mut [u8]) -> usize {
        0
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        if let Ok(s) = core::str::from_utf8(buf) {
            if s.trim() == "poweroff" {
                power::poweroff();
            } else if s.trim() == "reboot" {
                power::reboot();
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
    if path == "/power" {
        Ok(Arc::new(Mutex::new(PowerDevice)))
    } else {
        Err(())
    }
}
