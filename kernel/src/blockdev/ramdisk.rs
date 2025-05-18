use fatfs::{Error, IoBase, Read, Seek, SeekFrom, Write};

pub struct RamDisk {
    size_in_blocks: usize,
    ptr: *mut u8,
    cur_pos: usize,
}

impl RamDisk {
    pub fn with_addr_and_size(addr: *mut u8, size: u64) -> Self {
        Self {
            size_in_blocks: size.div_ceil(512) as usize,
            ptr: addr,
            cur_pos: 0,
        }
    }
}

unsafe impl Sync for RamDisk {}
unsafe impl Send for RamDisk {}

impl IoBase for RamDisk {
    type Error = Error<()>;
}

impl Read for RamDisk {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let will_read = if buf.len() < 512 { buf.len() } else { 512 };
        if self.cur_pos + will_read < self.size_in_blocks * 512 {
            unsafe {
                core::ptr::copy(self.ptr.add(self.cur_pos), buf.as_mut_ptr(), will_read);
            }
            self.cur_pos += will_read;
            Ok(will_read)
        } else {
            Err(Error::Io(()))
        }
    }
}

impl Write for RamDisk {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let will_write = if buf.len() < 512 { buf.len() } else { 512 };
        if self.cur_pos <= self.size_in_blocks * 512 {
            unsafe {
                core::ptr::copy(buf.as_ptr(), self.ptr.add(self.cur_pos), will_write);
            }
            self.cur_pos += will_write;
            Ok(will_write)
        } else {
            Err(Error::Io(()))
        }
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl Seek for RamDisk {
    fn seek(&mut self, frm: SeekFrom) -> Result<u64, Self::Error> {
        let new_pos = match frm {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => (self.size_in_blocks * 512) as i64 + offset,
            SeekFrom::Current(offset) => self.cur_pos as i64 + offset,
        };
        if new_pos < 0 || new_pos > (self.size_in_blocks * 512) as i64 {
            Err(Error::Io(()))
        } else {
            self.cur_pos = new_pos as usize;
            Ok(self.cur_pos as u64)
        }
    }
}
