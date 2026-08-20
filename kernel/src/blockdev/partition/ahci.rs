pub type AhciPartition = super::Partition<crate::blockdev::ahci::AhciBlockDevice>;

use gpt_disk_io::{
    BlockIo,
    gpt_disk_types::{BlockSize, Lba},
};

impl BlockIo for crate::blockdev::ahci::AhciBlockDevice {
    type Error = bool;

    fn block_size(&self) -> BlockSize {
        BlockSize::BS_512
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.identify.block_count)
    }

    fn read_blocks(&mut self, start_lba: Lba, output: &mut [u8]) -> Result<(), Self::Error> {
        if !output
            .len()
            .is_multiple_of(crate::blockdev::ahci::BLOCK_SIZE)
        {
            return Err(true);
        }
        let mut device = self.device.lock();
        for (sector, chunk) in (start_lba.to_u64()..).zip(
            output
                .as_chunks_mut::<{ crate::blockdev::ahci::BLOCK_SIZE }>()
                .0
                .iter_mut(),
        ) {
            device.read_block(sector, chunk);
        }
        Ok(())
    }

    fn write_blocks(&mut self, _start_lba: Lba, _input: &[u8]) -> Result<(), Self::Error> {
        Err(true)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
