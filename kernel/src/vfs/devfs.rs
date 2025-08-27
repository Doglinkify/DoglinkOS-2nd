use super::{VfsDirectory, VfsFile};
use alloc::sync::Arc;
use spin::Mutex;

pub(super) fn get_fs<T>(_device: Option<T>) -> Arc<dyn VfsDirectory>
where
    T: fatfs::ReadWriteSeek + Send + 'static,
{
    Arc::new(DevFileSystem)
}

struct DevFileSystem;

struct StdoutDevice;

impl VfsFile for StdoutDevice {
    fn size(&mut self) -> usize {
        0
    }

    fn read(&mut self, _buf: &mut [u8]) -> usize {
        0
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        crate::console::TERMINAL.lock().process(buf);
        buf.len()
    }

    fn seek(&mut self, _pos: super::SeekFrom) -> usize {
        0
    }
}

struct StderrDevice;

impl VfsFile for StderrDevice {
    fn size(&mut self) -> usize {
        0
    }

    fn read(&mut self, _buf: &mut [u8]) -> usize {
        0
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        let mut terminal = crate::console::TERMINAL.lock();
        terminal.process(b"\x1b[31m");
        terminal.process(buf);
        terminal.process(b"\x1b[0m");
        buf.len()
    }

    fn seek(&mut self, _pos: super::SeekFrom) -> usize {
        0
    }
}

impl VfsDirectory for DevFileSystem {
    fn file(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()> {
        if path.starts_with("/disk") {
            Ok(Arc::new(Mutex::new(
                crate::blockdev::ahci::AHCI
                    .iter()
                    .nth(path[5..].parse().map_err(|_| ())?)
                    .ok_or(())?,
            )))
        } else if path.starts_with("/nvme") {
            let res = path.find('-').ok_or(())?;
            let device = path[5..res].parse::<usize>().map_err(|_| ())?;
            let namespace = path[(res + 1)..].parse::<usize>().map_err(|_| ())?;
            Ok(Arc::new(Mutex::new({
                let v = crate::blockdev::nvme::NVME.iter().nth(device).ok_or(())?;
                v[namespace].clone()
            })))
        } else if path == "/initrd" {
            let file = super::MODULE_REQUEST.get_response().unwrap().modules()[0];
            Ok(Arc::new(Mutex::new(
                crate::blockdev::ramdisk::RamDisk::with_addr_and_size(file.addr(), file.size()),
            )))
        } else if path == "/stdout" {
            Ok(Arc::new(Mutex::new(StdoutDevice)))
        } else if path == "/stderr" {
            Ok(Arc::new(Mutex::new(StderrDevice)))
        } else {
            Err(())
        }
    }

    fn create_file_or_open_existing(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()> {
        self.file(path)
    }

    fn remove(&self, _path: &str) -> bool {
        false
    }
}
