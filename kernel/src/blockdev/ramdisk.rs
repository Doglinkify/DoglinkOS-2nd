use alloc::alloc::{alloc, dealloc, Layout};
use crate::println;
use fatfs::{format_volume, FormatVolumeOptions, FileSystem, FsOptions, IoBase, Read, Write, Seek, Error, SeekFrom};

pub struct RamDisk {
    size_in_blocks: usize,
    layout: Layout,
    ptr: *mut u8,
    cur_pos: usize,
}

impl RamDisk {
    pub fn new(size_in_blocks: usize) -> Self {
        unsafe {
            let layout = Layout::from_size_align(size_in_blocks * 512, 4).unwrap();
            let allocated = alloc(layout);
            Self {
                size_in_blocks,
                layout,
                ptr: allocated,
                cur_pos: 0,
            }
        }
    }
}

impl Drop for RamDisk {
    fn drop(&mut self) {
        unsafe {
//            println!("{:#?} {:?}", self.layout, self.ptr);
            dealloc(self.ptr, self.layout);
        }
    }
}

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
        let new_pos: i64;
        match frm {
            SeekFrom::Start(offset) => {
                new_pos = offset as i64;
            },
            SeekFrom::End(offset) => {
                new_pos = (self.size_in_blocks * 512) as i64 + offset;
            },
            SeekFrom::Current(offset) => {
                new_pos = self.cur_pos as i64 + offset;
            }
        }
        if new_pos < 0 || new_pos > (self.size_in_blocks * 512) as i64 {
            Err(Error::Io(()))
        } else {
            self.cur_pos = new_pos as usize;
            Ok(self.cur_pos as u64)
        }
    }
}

pub fn test() {
    let mut ramdisk = RamDisk::new(128);
    format_volume(&mut ramdisk, FormatVolumeOptions::new()).expect("format volume failed");
    let fs = FileSystem::new(ramdisk, FsOptions::new()).expect("create fs failed");
    let root_dir = fs.root_dir();
    macro_rules! file_content {
        ($name:expr, $content:expr) => {
            {
                let mut file = root_dir.create_file($name).expect("cre");
                file.write_all($content).unwrap();
            }
        }
    }
    file_content!("zzjrabbit.txt", b"Hello, FAT!");
    file_content!("text.txt", b"ADG");
    for f in root_dir.iter() {
        let e = f.unwrap();
        println!("[DEBUG] ramdisk: Name: {}, Size: {}", e.file_name(), e.len());
    }
//    println!("{:#?}", root_dir);
}
