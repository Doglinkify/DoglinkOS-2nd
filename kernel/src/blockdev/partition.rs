use fatfs::SeekFrom;
use gpt_disk_io::{
    gpt_disk_types::{BlockSize, GptPartitionEntry},
    BlockIo, Disk,
};

use crate::{
    blockdev::ahci::{AhciBlockDevice, BLOCK_SIZE},
    println,
};

impl BlockIo for AhciBlockDevice {
    type Error = bool;

    fn block_size(&self) -> BlockSize {
        BlockSize::BS_512
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.identify.block_count)
    }

    fn read_blocks(
        &mut self,
        start_lba: gpt_disk_io::gpt_disk_types::Lba,
        mut dst: &mut [u8],
    ) -> Result<(), Self::Error> {
        let mut device = self.device.lock();
        let mut sector = start_lba.to_u64();
        while !dst.is_empty() {
            device.read_block(sector, &mut dst[..512]);
            dst = &mut dst[512..];
            sector += 1;
        }
        Ok(())
    }

    fn write_blocks(
        &mut self,
        start_lba: gpt_disk_io::gpt_disk_types::Lba,
        mut src: &[u8],
    ) -> Result<(), Self::Error> {
        let mut device = self.device.lock();
        let mut sector = start_lba.to_u64();
        while !src.is_empty() {
            device.write_block(sector, &src[..512]);
            src = &src[512..];
            sector += 1;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub struct AhciPartition {
    block_device: AhciBlockDevice,
    partition_entry: GptPartitionEntry,
}

impl AhciPartition {
    pub fn new(mut block_device: AhciBlockDevice, part_number: usize) -> Self {
        let mut disk = Disk::new(block_device.clone()).unwrap();
        let mut block_buf = [0; 512];
        let primary_header = disk.read_primary_gpt_header(&mut block_buf).unwrap();
        assert!(primary_header.is_signature_valid());
        let layout = primary_header.get_partition_entry_array_layout().unwrap();
        let partition_entry = disk
            .gpt_partition_entry_array_iter(layout, &mut block_buf)
            .unwrap()
            .nth(part_number)
            .unwrap()
            .unwrap();
        let _ = fatfs::Seek::seek(
            &mut block_device,
            SeekFrom::Start(partition_entry.starting_lba.to_u64() * (BLOCK_SIZE as u64)),
        );
        Self {
            block_device,
            partition_entry,
        }
    }
}

impl fatfs::IoBase for AhciPartition {
    type Error = ();
}

impl fatfs::Read for AhciPartition {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        fatfs::Read::read(&mut self.block_device, buf)
    }
}

impl fatfs::Write for AhciPartition {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        fatfs::Write::write(&mut self.block_device, buf)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        fatfs::Write::flush(&mut self.block_device)
    }
}

impl fatfs::Seek for AhciPartition {
    fn seek(&mut self, pos: fatfs::SeekFrom) -> Result<u64, Self::Error> {
        fatfs::Seek::seek(
            &mut self.block_device,
            match pos {
                SeekFrom::Start(x) => SeekFrom::Start(
                    self.partition_entry.starting_lba.to_u64() * (BLOCK_SIZE as u64) + x,
                ),
                SeekFrom::End(x) => SeekFrom::End(
                    (self.partition_entry.ending_lba.to_u64() as i64 + 1) * (BLOCK_SIZE as i64) + x,
                ),
                abs => abs,
            },
        )
    }
}

pub fn test() {
    let block_device = crate::blockdev::ahci::AHCI.iter().nth(0).unwrap();
    let partition = AhciPartition::new(block_device, 0);
    // mount(Some(partition), "/mnt", crate::vfs::get_fat_fs);
    // let file = crate::vfs::get_file("/mnt/kernel").unwrap();
    let fs = crate::vfs::get_fat_fs(Some(partition));
    let file = fs.file("kernel").unwrap();
    let mut buf = [0; 128];
    let mut file = file.lock();
    file.read_exact(&mut buf);
    println!("[DEBUG] blockdev/partition/test: the first 128 bytes of the kernel image is {buf:?}");
}
