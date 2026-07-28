use alloc::sync::Arc;
use spin::Mutex;

use crate::vfs::VfsFile;

pub(super) fn open(path: &str) -> Result<Arc<Mutex<dyn VfsFile>>, ()> {
    if !path.starts_with("/nvme") {
        return Err(());
    }

    let res = path.find('-').ok_or(())?;
    let device = path[5..res].parse::<usize>().map_err(|_| ())?;
    let namespace = path[(res + 1)..].parse::<usize>().map_err(|_| ())?;
    Ok(Arc::new(Mutex::new({
        let v = crate::blockdev::nvme::NVME.iter().nth(device).ok_or(())?;
        v[namespace].clone()
    })))
}
