//! Read-only USB mass-storage block devices exposed through devfs.

use alloc::{string::String, sync::Arc, vec::Vec};
use spin::{Lazy, Mutex};

const BLOCK_SIZE: usize = 512;

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
}

impl crate::vfs::VfsFile for UsbBlockDevice {
    fn size(&mut self) -> usize {
        self.blocks.saturating_mul(BLOCK_SIZE as u64) as usize
    }

    fn read(&mut self, output: &mut [u8]) -> usize {
        let end = self
            .position
            .checked_add(output.len())
            .unwrap_or(usize::MAX);
        if end > self.size() || record(self.id).is_none_or(|device| !device.online) {
            return 0;
        }
        let mut done = 0;
        while done < output.len() {
            let offset = self.position + done;
            let lba = (offset / BLOCK_SIZE) as u64;
            let within = offset % BLOCK_SIZE;
            let take = (BLOCK_SIZE - within).min(output.len() - done);
            let mut sector = [0u8; BLOCK_SIZE];
            if !crate::xhci::read_usb_blocks(self.id, lba, &mut sector) {
                return done;
            }
            output[done..done + take].copy_from_slice(&sector[within..within + take]);
            done += take;
        }
        self.position += done;
        done
    }

    fn write(&mut self, _buf: &[u8]) -> usize {
        0
    }

    fn seek(&mut self, pos: crate::vfs::SeekFrom) -> usize {
        let size = self.size() as i64;
        let next = match pos {
            crate::vfs::SeekFrom::Start(value) => value as i64,
            crate::vfs::SeekFrom::End(value) => size.saturating_add(value as i64),
            crate::vfs::SeekFrom::Current(value) => {
                (self.position as i64).saturating_add(value as i64)
            }
        };
        if next >= 0 && next <= size {
            self.position = next as usize;
        }
        self.position
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
