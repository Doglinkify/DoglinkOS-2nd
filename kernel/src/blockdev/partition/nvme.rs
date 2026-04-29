use core::alloc::Layout;

use alloc::alloc::{alloc, dealloc};
use fatfs::SeekFrom;
use gpt_disk_io::{
    gpt_disk_types::{BlockSize, GptPartitionEntry},
    BlockIo, Disk,
};

use crate::{blockdev::nvme::NvmeBlockDevice, println, vfs::mount};

impl BlockIo for NvmeBlockDevice {
    type Error = bool;

    fn block_size(&self) -> BlockSize {
        BlockSize::from_usize(self.namespace.block_size() as usize).unwrap()
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.namespace.block_count())
    }

    fn read_blocks(
        &mut self,
        start_lba: gpt_disk_io::gpt_disk_types::Lba,
        mut dst: &mut [u8],
    ) -> Result<(), Self::Error> {
        let t = self.qpairs.first_entry().unwrap();
        let mut qp = t.get().lock();
        let block_size = self.namespace.block_size() as usize;
        let buf2 = unsafe { alloc(Layout::from_size_align(block_size, block_size).unwrap()) };
        let mut sector = start_lba.to_u64();
        while !dst.is_empty() {
            let _ = qp.read(buf2, block_size, sector);
            qp.flush().unwrap();
            unsafe {
                core::ptr::copy(buf2, dst.as_mut_ptr(), block_size);
            }
            dst = &mut dst[block_size..];
            sector += 1;
        }
        unsafe {
            dealloc(
                buf2,
                Layout::from_size_align(block_size, block_size).unwrap(),
            );
        }
        Ok(())
    }

    fn write_blocks(
        &mut self,
        _start_lba: gpt_disk_io::gpt_disk_types::Lba,
        mut _src: &[u8],
    ) -> Result<(), Self::Error> {
        // let t = self.qpairs.first_entry().unwrap();
        // let mut qp = t.get().lock();
        // let block_size = self.namespace.block_size() as usize;
        // let buf2 = unsafe { alloc(Layout::from_size_align(block_size, block_size).unwrap()) };
        // let mut sector = start_lba.to_u64();
        // while !src.is_empty() {
        //     unsafe {
        //         core::ptr::copy(src.as_ptr(), buf2, block_size);
        //     }
        //     qp.write(buf2, block_size, sector);
        //     src = &src[block_size..];
        //     sector += 1;
        // }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub struct NvmePartition {
    block_device: NvmeBlockDevice,
    partition_entry: GptPartitionEntry,
}

impl NvmePartition {
    pub fn new(mut block_device: NvmeBlockDevice, part_number: usize) -> Self {
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
        let block_size = block_device.namespace.block_size();
        let _ = fatfs::Seek::seek(
            &mut block_device,
            SeekFrom::Start(partition_entry.starting_lba.to_u64() * (block_size as u64)),
        );
        Self {
            block_device,
            partition_entry,
        }
    }
}

impl fatfs::IoBase for NvmePartition {
    type Error = ();
}

impl fatfs::Read for NvmePartition {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        fatfs::Read::read(&mut self.block_device, buf)
    }
}

impl fatfs::Write for NvmePartition {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        fatfs::Write::write(&mut self.block_device, buf)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        fatfs::Write::flush(&mut self.block_device)
    }
}

impl fatfs::Seek for NvmePartition {
    fn seek(&mut self, pos: fatfs::SeekFrom) -> Result<u64, Self::Error> {
        let block_size = self.block_device.namespace.block_size();
        fatfs::Seek::seek(
            &mut self.block_device,
            match pos {
                SeekFrom::Start(x) => SeekFrom::Start(
                    self.partition_entry.starting_lba.to_u64() * (block_size as u64) + x,
                ),
                SeekFrom::End(x) => SeekFrom::End(
                    (self.partition_entry.ending_lba.to_u64() as i64 + 1) * (block_size as i64) + x,
                ),
                abs => abs,
            },
        )
    }
}

pub fn test() {
    let block_device = crate::blockdev::nvme::NVME
        .iter()
        .nth(0)
        .unwrap()
        .into_iter()
        .nth(0)
        .unwrap();
    let partition = NvmePartition::new(block_device, 0);
    mount(Some(partition), "/mnt/", crate::vfs::get_fat_fs);
    let file = crate::vfs::get_file("/mnt/kernel").unwrap();
    let mut buf = [0; 128];
    let mut file = file.lock();
    file.read_exact(&mut buf);
    println!("[DEBUG] blockdev/partition/test: the first 128 bytes of the kernel image is {buf:?}");
}
