//! Read-only USB mass-storage block devices exposed through devfs.

use alloc::{string::String, sync::Arc, vec::Vec};
use core::fmt;
use gpt_disk_io::{
    BlockIo,
    gpt_disk_types::{BlockSize, Lba},
};
use spin::{Lazy, Mutex};

const BLOCK_SIZE: usize = 512;

/// A device removal or transport failure observed by the USB block layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbError {
    Offline,
    OutOfRange,
    InvalidBuffer,
    ReadFailed,
    ReadOnly,
}

impl fmt::Display for UsbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Offline => "USB storage device removed",
            Self::OutOfRange => "USB storage request outside device",
            Self::InvalidBuffer => "USB storage request is not sector aligned",
            Self::ReadFailed => "USB storage transfer failed",
            Self::ReadOnly => "USB storage is read-only",
        })
    }
}

impl fatfs::IoError for UsbError {
    fn is_interrupted(&self) -> bool {
        false
    }

    fn new_unexpected_eof_error() -> Self {
        Self::OutOfRange
    }

    fn new_write_zero_error() -> Self {
        Self::ReadOnly
    }
}

#[derive(Clone, Copy)]
struct Record {
    id: usize,
    blocks: u64,
    online: bool,
}

static DEVICES: Lazy<Mutex<Vec<Record>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub fn register(blocks: u64) -> usize {
    let mut devices = DEVICES.lock();
    let id = devices
        .iter()
        .map(|device| device.id)
        .max()
        .map_or(0, |id| id + 1);
    devices.push(Record {
        id,
        blocks,
        online: true,
    });
    crate::println!(
        "[INFO] blockdev: usb{} registered, blocks {}, block size {}",
        id,
        blocks,
        BLOCK_SIZE
    );
    id
}

pub fn offline(id: usize) {
    if let Some(device) = DEVICES.lock().iter_mut().find(|device| device.id == id) {
        device.online = false;
        crate::println!("[INFO] blockdev: usb{} offline", id);
    }
}

pub fn online_devices() -> Vec<(usize, u64)> {
    DEVICES
        .lock()
        .iter()
        .filter(|device| device.online)
        .map(|device| (device.id, device.blocks))
        .collect()
}

fn record(id: usize) -> Option<Record> {
    DEVICES
        .lock()
        .iter()
        .find(|device| device.id == id)
        .copied()
}

#[derive(Clone)]
pub struct UsbBlockDevice {
    id: usize,
    blocks: u64,
    position: usize,
}

impl UsbBlockDevice {
    pub fn open(id: usize) -> Option<Self> {
        let device = record(id)?;
        device.online.then_some(Self {
            id,
            blocks: device.blocks,
            position: 0,
        })
    }

    pub fn is_online(&self) -> bool {
        record(self.id).is_some_and(|device| device.online)
    }

    fn capacity_bytes(&self) -> usize {
        self.blocks.saturating_mul(BLOCK_SIZE as u64) as usize
    }

    fn read_bytes(&mut self, output: &mut [u8]) -> Result<usize, UsbError> {
        if !self.is_online() {
            return Err(UsbError::Offline);
        }
        let end = self
            .position
            .checked_add(output.len())
            .ok_or(UsbError::OutOfRange)?;
        if end > self.capacity_bytes() {
            return Err(UsbError::OutOfRange);
        }
        let mut done = 0;
        while done < output.len() {
            if !self.is_online() {
                return Err(UsbError::Offline);
            }
            let offset = self.position + done;
            let lba = (offset / BLOCK_SIZE) as u64;
            let within = offset % BLOCK_SIZE;
            let take = (BLOCK_SIZE - within).min(output.len() - done);
            let mut sector = [0u8; BLOCK_SIZE];
            if !crate::xhci::read_usb_blocks(self.id, lba, &mut sector) {
                return Err(if self.is_online() {
                    UsbError::ReadFailed
                } else {
                    UsbError::Offline
                });
            }
            output[done..done + take].copy_from_slice(&sector[within..within + take]);
            done += take;
        }
        self.position = end;
        Ok(done)
    }
}

impl fatfs::IoBase for UsbBlockDevice {
    type Error = UsbError;
}

impl fatfs::Read for UsbBlockDevice {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, Self::Error> {
        self.read_bytes(output)
    }
}

impl fatfs::Write for UsbBlockDevice {
    fn write(&mut self, _buf: &[u8]) -> Result<usize, Self::Error> {
        Err(UsbError::ReadOnly)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        if self.is_online() {
            Ok(())
        } else {
            Err(UsbError::Offline)
        }
    }
}

impl fatfs::Seek for UsbBlockDevice {
    fn seek(&mut self, pos: fatfs::SeekFrom) -> Result<u64, Self::Error> {
        if !self.is_online() {
            return Err(UsbError::Offline);
        }
        let size = self.capacity_bytes() as i64;
        let next = match pos {
            fatfs::SeekFrom::Start(value) => value as i64,
            fatfs::SeekFrom::End(value) => size.saturating_add(value),
            fatfs::SeekFrom::Current(value) => (self.position as i64).saturating_add(value),
        };
        if !(0..=size).contains(&next) {
            return Err(UsbError::OutOfRange);
        }
        self.position = next as usize;
        Ok(self.position as u64)
    }
}

impl BlockIo for UsbBlockDevice {
    type Error = UsbError;

    fn block_size(&self) -> BlockSize {
        BlockSize::BS_512
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        if self.is_online() {
            Ok(self.blocks)
        } else {
            Err(UsbError::Offline)
        }
    }

    fn read_blocks(&mut self, start_lba: Lba, output: &mut [u8]) -> Result<(), Self::Error> {
        if output.len() % BLOCK_SIZE != 0 {
            return Err(UsbError::InvalidBuffer);
        }
        let start = start_lba
            .to_u64()
            .checked_mul(BLOCK_SIZE as u64)
            .ok_or(UsbError::OutOfRange)?;
        let end = start
            .checked_add(output.len() as u64)
            .ok_or(UsbError::OutOfRange)?;
        if end > self.blocks.saturating_mul(BLOCK_SIZE as u64) {
            return Err(UsbError::OutOfRange);
        }
        self.position = start as usize;
        self.read_bytes(output).map(|_| ())
    }

    fn write_blocks(&mut self, _start_lba: Lba, _input: &[u8]) -> Result<(), Self::Error> {
        Err(UsbError::ReadOnly)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        fatfs::Write::flush(self)
    }
}

impl crate::vfs::VfsFile for UsbBlockDevice {
    fn size(&mut self) -> usize {
        self.capacity_bytes()
    }

    fn read(&mut self, output: &mut [u8]) -> usize {
        fatfs::Read::read(self, output).unwrap_or(0)
    }

    fn write(&mut self, _buf: &[u8]) -> usize {
        0
    }

    fn seek(&mut self, pos: crate::vfs::SeekFrom) -> usize {
        fatfs::Seek::seek(
            self,
            match pos {
                crate::vfs::SeekFrom::Start(value) => fatfs::SeekFrom::Start(value as u64),
                crate::vfs::SeekFrom::End(value) => fatfs::SeekFrom::End(value as i64),
                crate::vfs::SeekFrom::Current(value) => fatfs::SeekFrom::Current(value as i64),
            },
        )
        .map(|value| value as usize)
        .unwrap_or(0)
    }
}

pub fn open(path: &str) -> Result<Arc<Mutex<dyn crate::vfs::VfsFile>>, ()> {
    let id = path
        .strip_prefix("/usb")
        .ok_or(())?
        .parse::<usize>()
        .map_err(|_| ())?;
    Ok(Arc::new(Mutex::new(UsbBlockDevice::open(id).ok_or(())?)))
}

pub fn name(id: usize) -> String {
    alloc::format!("usb{id}")
}
