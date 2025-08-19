use super::{VfsDirectory, VfsFile};
use alloc::sync::Arc;
use fatfs::{FileSystem, FsOptions, ReadWriteSeek};
use spin::Mutex;

pub(super) fn get_fs<T>(device: Option<T>) -> Arc<dyn VfsDirectory>
where
    T: fatfs::ReadWriteSeek + Send + 'static,
{
    Arc::new(WrappedFileSystem(
        FileSystem::new(device.unwrap(), FsOptions::new()).unwrap(),
    ))
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
        let mut res = 0;
        for extent in self.0.extents() {
            res += extent.unwrap().size as usize;
        }
        res
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        use fatfs::Read;
        self.0.read(buf).unwrap()
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        use fatfs::Write;
        self.0.write(buf).unwrap()
    }

    fn seek(&mut self, pos: crate::vfs::SeekFrom) -> usize {
        use fatfs::Seek;
        self.0
            .seek(match pos {
                crate::vfs::SeekFrom::End(x) => fatfs::SeekFrom::End(x as i64),
                crate::vfs::SeekFrom::Current(x) => fatfs::SeekFrom::Current(x as i64),
                crate::vfs::SeekFrom::Start(x) => fatfs::SeekFrom::Start(x as u64),
            })
            .unwrap() as usize
    }
}
