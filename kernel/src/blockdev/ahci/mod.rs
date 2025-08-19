use alloc::{sync::Arc, vec::Vec};
use identify::IdentifyData;
use spin::{Lazy, Mutex};
use x86_64::VirtAddr;

use crate::mm::phys_to_virt;

pub mod cmd;
pub mod driver;
pub mod hba;
pub mod identify;

pub use driver::{Ahci, BLOCK_SIZE};
pub use hba::HbaMemory;

pub struct AhciBlockDevice {
    pub device: Arc<Mutex<Ahci>>,
    pub identify: IdentifyData,
    cur_pos: usize,
}

impl fatfs::IoBase for AhciBlockDevice {
    type Error = ();
}

impl fatfs::Read for AhciBlockDevice {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let mut device = self.device.lock();
        let size = buf.len();
        let sector = self.cur_pos / BLOCK_SIZE;
        let mut buf2 = [0; BLOCK_SIZE];
        device.read_block(sector as u64, &mut buf2);
        let (t1, t2) = match self.cur_pos % BLOCK_SIZE {
            0 => (BLOCK_SIZE, 0),
            o => (o, BLOCK_SIZE - o),
        };
        let will_read = core::cmp::min(size, t1);
        buf[..will_read].copy_from_slice(&buf2[t2..(t2 + will_read)]);
        Ok(will_read)
    }
}

impl fatfs::Write for AhciBlockDevice {
    fn write(&mut self, _buf: &[u8]) -> Result<usize, Self::Error> {
        Err(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl fatfs::Seek for AhciBlockDevice {
    fn seek(&mut self, pos: fatfs::SeekFrom) -> Result<u64, Self::Error> {
        let new_pos = match pos {
            fatfs::SeekFrom::Start(offset) => offset as i64,
            fatfs::SeekFrom::End(offset) => {
                (self.identify.block_count as usize * BLOCK_SIZE) as i64 + offset
            }
            fatfs::SeekFrom::Current(offset) => self.cur_pos as i64 + offset,
        };
        if new_pos < 0 || new_pos > (self.identify.block_count as usize * BLOCK_SIZE) as i64 {
            Err(())
        } else {
            self.cur_pos = new_pos as usize;
            Ok(self.cur_pos as u64)
        }
    }
}

impl crate::vfs::VfsFile for AhciBlockDevice {
    fn size(&mut self) -> usize {
        self.identify.block_count as usize * 512
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        <Self as fatfs::Read>::read(self, buf).unwrap()
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        <Self as fatfs::Write>::write(self, buf).unwrap()
    }

    fn seek(&mut self, pos: crate::vfs::SeekFrom) -> usize {
        <Self as fatfs::Seek>::seek(
            self,
            match pos {
                crate::vfs::SeekFrom::End(x) => fatfs::SeekFrom::End(x as i64),
                crate::vfs::SeekFrom::Current(x) => fatfs::SeekFrom::Current(x as i64),
                crate::vfs::SeekFrom::Start(x) => fatfs::SeekFrom::Start(x as u64),
            },
        )
        .unwrap() as usize
    }
}

impl AhciManager {
    pub fn iter(&self) -> impl Iterator<Item = AhciBlockDevice> + use<'_> {
        self.0.iter().map(|device| AhciBlockDevice {
            device: device.clone(),
            identify: device.lock().identity(),
            cur_pos: 0,
        })
    }
}

pub struct AhciManager(Vec<Arc<Mutex<Ahci>>>);

pub static AHCI: Lazy<AhciManager> = Lazy::new(|| {
    let mut connections = Vec::new();

    crate::pcie::enumrate::doit(|_, _, _, device| {
        if device.class_code == 1 && device.subclass == 6 {
            let virtual_address = phys_to_virt((device.bar[5] & 0xfffffff0u32) as u64);

            for ahci_device in Ahci::new(VirtAddr::new(virtual_address)) {
                connections.push(Arc::new(Mutex::new(ahci_device)));
            }
        }
    });

    AhciManager(connections)
});

pub fn init() {
    for ahci in AHCI.iter() {
        let res = crate::mm::convert_unit(ahci.identify.block_count * BLOCK_SIZE as u64);
        crate::println!(
            "[INFO] blockdev: achi: found {}, size = {} {}",
            ahci.identify.model_number,
            res.0,
            res.1
        );
    }
}
