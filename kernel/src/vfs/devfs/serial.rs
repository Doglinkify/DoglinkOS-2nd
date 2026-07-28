use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use spin::Mutex;

use crate::vfs::{SeekFrom, VfsFile};

struct SerialDevice;

impl VfsFile for SerialDevice {
    fn size(&mut self) -> usize {
        0
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        if !crate::console::serial::SERIAL_OK.load(Ordering::Relaxed) {
            return 0;
        }
        let mut count = 0;
        while count < buf.len() {
            match crate::console::serial::read() {
                Some(b) => {
                    buf[count] = b;
                    count += 1;
                }
                None => break,
            }
        }
        count
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        if !crate::console::serial::SERIAL_OK.load(Ordering::Relaxed) {
            return buf.len();
        }
        crate::console::serial::write_bytes(buf);
        buf.len()
    }

    fn seek(&mut self, _pos: SeekFrom) -> usize {
        0
    }
}

pub(super) fn open(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    if path == "/serial" {
        Ok(Arc::new(Mutex::new(SerialDevice)))
    } else {
        Err(())
    }
}
