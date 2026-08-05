use alloc::sync::Arc;
use spin::Mutex;

use crate::vfs::{MODULE_REQUEST, VfsFile};

pub(super) fn open(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    if path != "/initrd" {
        return Err(());
    }

    let file = MODULE_REQUEST.response().unwrap().modules()[0];
    let data = file.data();
    Ok(Arc::new(Mutex::new(
        crate::blockdev::ramdisk::RamDisk::with_addr_and_size(
            data.as_ptr() as *mut u8,
            data.len() as u64,
        ),
    )))
}
