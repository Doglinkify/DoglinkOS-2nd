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
mod tty;

use alloc::sync::Arc;

use super::VfsDirectory;

pub(super) fn get_fs<T>(_device: Option<T>) -> Result<Arc<dyn VfsDirectory>, ()>
where
    T: fatfs::ReadWriteSeek + Send + 'static,
{
    Ok(Arc::new(filesystem::DevFileSystem))
}
