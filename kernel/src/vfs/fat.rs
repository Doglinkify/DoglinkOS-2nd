use crate::vfs::{VfsDirectory, VfsFile};
use alloc::boxed::Box;
use fatfs::{FileSystem, FsOptions};

pub(super) fn get_fs<T>(device: T) -> Box<dyn VfsDirectory>
where
    T: fatfs::ReadWriteSeek + Send + 'static,
{
    Box::new(FileSystem::new(device, FsOptions::new()).unwrap())
}

impl<T: fatfs::ReadWriteSeek + Send> VfsDirectory for FileSystem<T> {
    fn file(&self, path: &str) -> Result<Box<dyn VfsFile + '_>, ()> {
        self.root_dir()
            .open_file(path)
            .map_err(|_| ())
            .map(move |x| Box::new(x) as _)
    }
}

impl<T: fatfs::ReadWriteSeek, TP: fatfs::TimeProvider, OCC> VfsFile
    for fatfs::File<'_, T, TP, OCC>
{
    fn size(&mut self) -> usize {
        let mut res = 0;
        for extent in self.extents() {
            res += extent.unwrap().size as usize;
        }
        res
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        <Self as fatfs::Read>::read(self, buf).unwrap()
    }

    fn write(&mut self, _buf: &mut [u8]) -> usize {
        0
    }

    fn seek(&mut self, _pos: super::SeekFrom) -> usize {
        0
    }
}
