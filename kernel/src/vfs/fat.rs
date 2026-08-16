use super::{DirEntry, SnapshotDirectory, VfsDirHandle, VfsDirectory, VfsFile};
use alloc::sync::Arc;
use alloc::vec::Vec;
use fatfs::{FileSystem, FsOptions, ReadWriteSeek};
use spin::Mutex;

pub fn get_fs<T>(device: Option<T>) -> Result<Arc<dyn VfsDirectory>, ()>
where
    T: fatfs::ReadWriteSeek + Send + 'static,
{
    let device = device.ok_or(())?;
    let filesystem = FileSystem::new(device, FsOptions::new()).map_err(|_| ())?;
    Ok(Arc::new(WrappedFileSystem(filesystem)))
}

pub struct WrappedFileSystem<T: ReadWriteSeek>(FileSystem<T>);

unsafe impl<T: ReadWriteSeek> Sync for WrappedFileSystem<T> {}

pub struct WrappedFile<'a, T: ReadWriteSeek, TP, OCC>(fatfs::File<'a, T, TP, OCC>);

unsafe impl<'a, T: ReadWriteSeek, TP, OCC> Send for WrappedFile<'a, T, TP, OCC> {}

impl<T: fatfs::ReadWriteSeek + Send> VfsDirectory for WrappedFileSystem<T> {
    fn file(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()> {
        self.0
            .root_dir()
            .open_file(path)
            .map_err(|_| ())
            .map(move |x| Arc::new(Mutex::new(WrappedFile(x))) as _)
    }

    fn directory(&self, path: &str) -> Result<Arc<Mutex<dyn VfsDirHandle + '_>>, ()> {
        let dir = if path == "/" || path.is_empty() {
            self.0.root_dir()
        } else {
            self.0.root_dir().open_dir(path).map_err(|_| ())?
        };
        let mut entries = Vec::new();
        for entry in dir.iter() {
            let entry = entry.map_err(|_| ())?;
            entries.push(DirEntry::new(entry.is_dir(), &entry.file_name()));
        }
        Ok(Arc::new(Mutex::new(SnapshotDirectory::new(entries))))
    }

    fn create_file_or_open_existing(&self, path: &str) -> Result<Arc<Mutex<dyn VfsFile + '_>>, ()> {
        self.0
            .root_dir()
            .create_file(path)
            .map_err(|_| ())
            .map(move |x| Arc::new(Mutex::new(WrappedFile(x))) as _)
    }

    fn remove(&self, path: &str) -> bool {
        self.0.root_dir().remove(path).is_ok()
    }
}

impl<T: fatfs::ReadWriteSeek, TP: fatfs::TimeProvider, OCC> VfsFile
    for WrappedFile<'_, T, TP, OCC>
{
    fn size(&mut self) -> usize {
        let mut res: usize = 0;
        for extent in self.0.extents() {
            let Ok(extent) = extent else {
                return 0;
            };
            res = res.saturating_add(extent.size as usize);
        }
        res
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        use fatfs::Read;
        self.0.read(buf).unwrap_or(0)
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        use fatfs::Write;
        self.0.write(buf).unwrap_or(0)
    }

    fn seek(&mut self, pos: crate::vfs::SeekFrom) -> usize {
        use fatfs::Seek;
        self.0
            .seek(match pos {
                crate::vfs::SeekFrom::End(x) => fatfs::SeekFrom::End(x as i64),
                crate::vfs::SeekFrom::Current(x) => fatfs::SeekFrom::Current(x as i64),
                crate::vfs::SeekFrom::Start(x) => fatfs::SeekFrom::Start(x as u64),
            })
            .unwrap_or(0) as usize
    }
}
