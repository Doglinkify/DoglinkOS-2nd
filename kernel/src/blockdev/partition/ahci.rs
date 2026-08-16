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
        if output.len() % crate::blockdev::ahci::BLOCK_SIZE != 0 {
            return Err(true);
        }
        let mut device = self.device.lock();
        let mut sector = start_lba.to_u64();
        for chunk in output.chunks_exact_mut(crate::blockdev::ahci::BLOCK_SIZE) {
            device.read_block(sector, chunk);
            sector += 1;
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
