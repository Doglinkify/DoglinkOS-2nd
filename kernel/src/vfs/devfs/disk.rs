use alloc::sync::Arc;
use spin::Mutex;

use crate::vfs::VfsFile;

pub(super) fn open(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    let Some(number) = path.strip_prefix("/disk") else {
        return Err(());
    };

    Ok(Arc::new(Mutex::new(
        crate::blockdev::ahci::AHCI
            .iter()
            .nth(number.parse().map_err(|_| ())?)
            .ok_or(())?,
    )))
}
