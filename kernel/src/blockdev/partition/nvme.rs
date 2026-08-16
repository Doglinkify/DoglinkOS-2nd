pub type NvmePartition = super::Partition<crate::blockdev::nvme::NvmeBlockDevice>;

use alloc::alloc::{alloc, dealloc};
use core::alloc::Layout;
use gpt_disk_io::{
    BlockIo,
    gpt_disk_types::{BlockSize, Lba},
};

impl BlockIo for crate::blockdev::nvme::NvmeBlockDevice {
    type Error = bool;

    fn block_size(&self) -> BlockSize {
        BlockSize::from_usize(self.namespace.block_size() as usize).unwrap_or(BlockSize::BS_512)
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.namespace.block_count())
    }

    fn read_blocks(&mut self, start_lba: Lba, output: &mut [u8]) -> Result<(), Self::Error> {
        let block_size = self.namespace.block_size() as usize;
        if output.len() % block_size != 0 {
            return Err(true);
        }
        let Some(entry) = self.qpairs.first_entry() else {
            return Err(true);
        };
        let layout = Layout::from_size_align(block_size, block_size).map_err(|_| true)?;
        let buffer = unsafe { alloc(layout) };
        if buffer.is_null() {
            return Err(true);
        }
        let mut qp = entry.get().lock();
        let mut sector = start_lba.to_u64();
        let result = (|| {
            for chunk in output.chunks_exact_mut(block_size) {
                qp.read(buffer, block_size, sector).map_err(|_| true)?;
                qp.flush().map_err(|_| true)?;
                unsafe {
                    core::ptr::copy_nonoverlapping(buffer, chunk.as_mut_ptr(), block_size);
                }
                sector += 1;
            }
            Ok(())
        })();
        unsafe {
            dealloc(buffer, layout);
        }
        result
    }

    fn write_blocks(&mut self, _start_lba: Lba, _input: &[u8]) -> Result<(), Self::Error> {
        Err(true)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
