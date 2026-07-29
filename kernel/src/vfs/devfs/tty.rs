use alloc::sync::Arc;
use spin::Mutex;

use crate::vfs::{SeekFrom, VfsFile};

struct TtyDevice;

impl VfsFile for TtyDevice {
    fn size(&mut self) -> usize {
        0
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        let mut count = 0;
        while count < buf.len() {
            match crate::console::INPUT_BUFFER.pop() {
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
        crate::console::TERMINAL.lock().process(buf);
        buf.len()
    }

    fn seek(&mut self, _pos: SeekFrom) -> usize {
        0
    }
}

pub(super) fn open(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    if matches!(path, "/tty" | "/terminal") {
        Ok(Arc::new(Mutex::new(TtyDevice)))
    } else {
        Err(())
    }
}
