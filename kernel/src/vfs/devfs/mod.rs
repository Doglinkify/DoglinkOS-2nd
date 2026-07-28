mod cmdline;
mod disk;
mod filesystem;
mod initrd;
mod nvme;
mod pcspk;
mod power;
mod serial;
mod stderr;
mod stdout;

use alloc::sync::Arc;

use super::VfsDirectory;

pub(super) fn get_fs<T>(_device: Option<T>) -> Arc<dyn VfsDirectory>
where
    T: fatfs::ReadWriteSeek + Send + 'static,
{
    Arc::new(filesystem::DevFileSystem)
}
