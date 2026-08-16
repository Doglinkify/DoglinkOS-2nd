//! Common GPT partition adapter for every block device.

pub mod ahci;
pub mod nvme;
pub mod usb;

use core::fmt;

use fatfs::SeekFrom;
use gpt_disk_io::{BlockIo, Disk};

/// A bounded, byte-addressable view of one GPT partition.
///
/// The underlying device remains responsible for reporting removal.  In
/// particular, a USB device returns its I/O error after it has gone offline;
/// the FAT layer therefore never accesses xHCI DMA memory after unplug.
pub struct Partition<T> {
    block_device: T,
    start: u64,
    end: u64,
    position: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartitionError {
    InvalidGpt,
    PartitionNotFound,
    UnsupportedBlockSize,
    OffsetOverflow,
    SeekFailed,
}

impl fmt::Display for PartitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidGpt => "invalid GPT",
            Self::PartitionNotFound => "GPT partition not found",
            Self::UnsupportedBlockSize => "unsupported block size",
            Self::OffsetOverflow => "partition offset overflow",
            Self::SeekFailed => "failed to seek to partition",
        })
    }
}

impl<T> Partition<T>
where
    T: BlockIo + Clone + fatfs::ReadWriteSeek,
{
    pub fn new(mut block_device: T, part_number: usize) -> Result<Self, PartitionError> {
        let block_size = block_device
            .block_size()
            .to_usize()
            .ok_or(PartitionError::UnsupportedBlockSize)? as u64;
        let mut disk = Disk::new(block_device.clone()).map_err(|_| PartitionError::InvalidGpt)?;
        let mut block_buf = [0; 4096];
        let header = disk
            .read_primary_gpt_header(&mut block_buf)
            .map_err(|_| PartitionError::InvalidGpt)?;
        if !header.is_signature_valid() {
            return Err(PartitionError::InvalidGpt);
        }
        let layout = header
            .get_partition_entry_array_layout()
            .map_err(|_| PartitionError::InvalidGpt)?;
        let partition_entry = disk
            .gpt_partition_entry_array_iter(layout, &mut block_buf)
            .map_err(|_| PartitionError::InvalidGpt)?
            .nth(part_number)
            .ok_or(PartitionError::PartitionNotFound)?
            .map_err(|_| PartitionError::InvalidGpt)?;
        let start = partition_entry
            .starting_lba
            .to_u64()
            .checked_mul(block_size)
            .ok_or(PartitionError::OffsetOverflow)?;
        let end = partition_entry
            .ending_lba
            .to_u64()
            .checked_add(1)
            .and_then(|lba| lba.checked_mul(block_size))
            .ok_or(PartitionError::OffsetOverflow)?;
        fatfs::Seek::seek(&mut block_device, SeekFrom::Start(start))
            .map_err(|_| PartitionError::SeekFailed)?;
        Ok(Self {
            block_device,
            start,
            end,
            position: 0,
        })
    }
}

type FatError<T> = fatfs::Error<<T as fatfs::IoBase>::Error>;

impl<T> fatfs::IoBase for Partition<T>
where
    T: BlockIo + Clone + fatfs::ReadWriteSeek,
{
    type Error = FatError<T>;
}

impl<T> fatfs::Read for Partition<T>
where
    T: BlockIo + Clone + fatfs::ReadWriteSeek,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let remaining = self
            .end
            .saturating_sub(self.start.saturating_add(self.position));
        let read_len = buf.len().min(remaining.min(usize::MAX as u64) as usize);
        let read = fatfs::Read::read(&mut self.block_device, &mut buf[..read_len])
            .map_err(fatfs::Error::Io)?;
        self.position = self.position.saturating_add(read as u64);
        Ok(read)
    }
}

impl<T> fatfs::Write for Partition<T>
where
    T: BlockIo + Clone + fatfs::ReadWriteSeek,
{
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        fatfs::Write::write(&mut self.block_device, buf).map_err(fatfs::Error::Io)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        fatfs::Write::flush(&mut self.block_device).map_err(fatfs::Error::Io)
    }
}

impl<T> fatfs::Seek for Partition<T>
where
    T: BlockIo + Clone + fatfs::ReadWriteSeek,
{
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        let next = match pos {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.end - self.start) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
        };
        let length = self.end - self.start;
        if next < 0 || next > i128::from(length) {
            return Err(fatfs::Error::InvalidInput);
        }
        let next = next as u64;
        let absolute = self
            .start
            .checked_add(next)
            .ok_or(fatfs::Error::InvalidInput)?;
        fatfs::Seek::seek(&mut self.block_device, SeekFrom::Start(absolute))
            .map_err(fatfs::Error::Io)?;
        self.position = next;
        Ok(next)
    }
}
