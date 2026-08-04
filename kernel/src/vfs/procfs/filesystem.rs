use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

use crate::vfs::{DirEntry, SeekFrom, VfsDirHandle, VfsDirectory, VfsFile};

pub(super) struct ProcFileSystem;

struct ProcDirectory {
    entries: Vec<DirEntry>,
    offset: usize,
}

struct ProcTextFile {
    data: String,
    pos: usize,
}

pub(crate) fn get_fs<T>(_: Option<T>) -> Arc<dyn VfsDirectory>
where
    T: fatfs::ReadWriteSeek,
{
    Arc::new(ProcFileSystem)
}

impl VfsDirectory for ProcFileSystem {
    fn file(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()> {
        if path == "/cmdline" {
            return Ok(Arc::new(Mutex::new(ProcTextFile {
                data: crate::cmdline::CMDLINE.clone(),
                pos: 0,
            })));
        }

        let (pid, file_name) = split_process_file_path(path)?;
        if file_name != "exe" {
            return Err(());
        }

        let data = {
            let tasks = crate::task::process::TASKS.lock();
            let task = tasks.get(pid).and_then(Option::as_ref).ok_or(())?;
            task.exe_path.clone().unwrap_or_default()
        };

        Ok(Arc::new(Mutex::new(ProcTextFile { data, pos: 0 })))
    }

    fn directory(&self, path: &str) -> Result<Arc<Mutex<dyn VfsDirHandle + '_>>, ()> {
        if path == "/" || path.is_empty() {
            let mut entries = vec![DirEntry::new(false, "cmdline")];
            let tasks = crate::task::process::TASKS.lock();
            for (pid, task) in tasks.iter().enumerate() {
                if task.is_some() {
                    entries.push(DirEntry::new(true, &format!("{pid}")));
                }
            }
            return Ok(Arc::new(Mutex::new(ProcDirectory { entries, offset: 0 })));
        }

        let pid = split_process_dir_path(path)?;
        let tasks = crate::task::process::TASKS.lock();
        if tasks.get(pid).and_then(Option::as_ref).is_none() {
            return Err(());
        }

        Ok(Arc::new(Mutex::new(ProcDirectory {
            entries: vec![DirEntry::new(false, "exe")],
            offset: 0,
        })))
    }

    fn create_file_or_open_existing(&self, _path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()> {
        Err(())
    }

    fn remove(&self, _path: &str) -> bool {
        false
    }
}

impl VfsDirHandle for ProcDirectory {
    fn getdents(&mut self, buf: &mut [DirEntry]) -> usize {
        let mut written = 0;
        while written < buf.len() && self.offset < self.entries.len() {
            buf[written] = self.entries[self.offset];
            self.offset += 1;
            written += 1;
        }
        written
    }
}

impl VfsFile for ProcTextFile {
    fn size(&mut self) -> usize {
        self.data.len()
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        let data = self.data.as_bytes();
        if self.pos >= data.len() {
            return 0;
        }
        let len = buf.len().min(data.len() - self.pos);
        buf[..len].copy_from_slice(&data[self.pos..self.pos + len]);
        self.pos += len;
        len
    }

    fn write(&mut self, _buf: &[u8]) -> usize {
        0
    }

    fn seek(&mut self, pos: SeekFrom) -> usize {
        let len = self.data.len();
        let new_pos = match pos {
            SeekFrom::Start(pos) => pos.min(len),
            SeekFrom::End(offset) => len.saturating_add_signed(offset).min(len),
            SeekFrom::Current(offset) => self.pos.saturating_add_signed(offset).min(len),
        };
        self.pos = new_pos;
        self.pos
    }
}

fn split_process_dir_path(path: &str) -> Result<usize, ()> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() || trimmed.contains('/') {
        return Err(());
    }
    trimmed.parse().map_err(|_| ())
}

fn split_process_file_path(path: &str) -> Result<(usize, &str), ()> {
    let trimmed = path.trim_matches('/');
    let Some((pid, file_name)) = trimmed.split_once('/') else {
        return Err(());
    };
    if file_name.contains('/') {
        return Err(());
    }
    Ok((pid.parse().map_err(|_| ())?, file_name))
}
