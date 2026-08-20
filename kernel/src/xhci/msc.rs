//! USB Mass Storage Bulk-Only Transport (BOT) and SCSI command layouts.
//!
//! The protocol code is deliberately independent of xHCI MMIO.  A controller
//! supplies the three bounded transfer operations through [`BotTransport`];
//! this makes every byte layout and recovery decision host-testable.

use super::usb::SetupRequest;

pub const CBW_LEN: usize = 31;
pub const CSW_LEN: usize = 13;
pub const BLOCK_SIZE: usize = 512;
pub const MAX_READ_BYTES: usize = 64 * 1024;

const CBW_SIGNATURE: u32 = 0x4342_5355;
const CSW_SIGNATURE: u32 = 0x5342_5355;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BotError {
    Transport,
    Stall,
    ShortTransfer,
    InvalidCswSignature,
    InvalidCswTag,
    InvalidResidue,
    CommandFailed,
    PhaseError,
    UnsupportedBlockSize(u32),
    Capacity16Required,
    ReadTooLarge,
    LbaOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Recovery {
    None,
    ResetAndClearHalts,
    Offline,
}

pub const fn recovery_for(error: BotError) -> Recovery {
    match error {
        BotError::Transport
        | BotError::Stall
        | BotError::ShortTransfer
        | BotError::InvalidCswSignature
        | BotError::InvalidCswTag
        | BotError::InvalidResidue
        | BotError::PhaseError => Recovery::ResetAndClearHalts,
        BotError::CommandFailed
        | BotError::UnsupportedBlockSize(_)
        | BotError::Capacity16Required
        | BotError::ReadTooLarge
        | BotError::LbaOverflow => Recovery::Offline,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataDirection {
    In,
    Out,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capacity {
    pub blocks: u64,
    pub block_size: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Csw {
    pub tag: u32,
    pub residue: u32,
    pub status: u8,
}

impl Csw {
    pub fn parse(bytes: &[u8]) -> Result<Self, BotError> {
        if bytes.len() != CSW_LEN {
            return Err(BotError::ShortTransfer);
        }
        if u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) != CSW_SIGNATURE {
            return Err(BotError::InvalidCswSignature);
        }
        let status = bytes[12];
        if status > 2 {
            return Err(BotError::PhaseError);
        }
        Ok(Self {
            tag: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            residue: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            status,
        })
    }

    pub fn validate(self, tag: u32, requested: usize) -> Result<(), BotError> {
        if self.tag != tag {
            return Err(BotError::InvalidCswTag);
        }
        if self.residue as usize > requested {
            return Err(BotError::InvalidResidue);
        }
        match self.status {
            0 if self.residue == 0 => Ok(()),
            0 => Err(BotError::InvalidResidue),
            1 => Err(BotError::CommandFailed),
            _ => Err(BotError::PhaseError),
        }
    }
}

pub fn cbw(
    tag: u32,
    transfer_len: u32,
    direction: DataDirection,
    lun: u8,
    cdb: &[u8],
) -> [u8; CBW_LEN] {
    let mut bytes = [0u8; CBW_LEN];
    bytes[..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
    bytes[4..8].copy_from_slice(&tag.to_le_bytes());
    bytes[8..12].copy_from_slice(&transfer_len.to_le_bytes());
    bytes[12] = if matches!(direction, DataDirection::In) {
        0x80
    } else {
        0
    };
    bytes[13] = lun;
    bytes[14] = cdb.len() as u8;
    let count = cdb.len().min(16);
    bytes[15..15 + count].copy_from_slice(&cdb[..count]);
    bytes
}

pub const fn get_max_lun(interface: u8) -> SetupRequest {
    SetupRequest {
        bm_request_type: 0xa1,
        request: 0xfe,
        value: 0,
        index: interface as u16,
        length: 1,
    }
}

pub const fn bot_reset(interface: u8) -> SetupRequest {
    SetupRequest {
        bm_request_type: 0x21,
        request: 0xff,
        value: 0,
        index: interface as u16,
        length: 0,
    }
}

pub const fn clear_halt(endpoint: u8) -> SetupRequest {
    SetupRequest {
        bm_request_type: 0x02,
        request: 1,
        value: 0,
        index: endpoint as u16,
        length: 0,
    }
}

pub const fn inquiry() -> [u8; 6] {
    [0x12, 0, 0, 0, 36, 0]
}
pub const fn test_unit_ready() -> [u8; 6] {
    [0, 0, 0, 0, 0, 0]
}
pub const fn request_sense() -> [u8; 6] {
    [0x03, 0, 0, 0, 18, 0]
}
pub const fn read_capacity10() -> [u8; 10] {
    [0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0]
}

pub fn read10(lba: u32, blocks: u16) -> [u8; 10] {
    let mut cdb = [0u8; 10];
    cdb[0] = 0x28;
    cdb[2..6].copy_from_slice(&lba.to_be_bytes());
    cdb[7..9].copy_from_slice(&blocks.to_be_bytes());
    cdb
}

pub fn parse_capacity10(bytes: &[u8]) -> Result<Capacity, BotError> {
    if bytes.len() != 8 {
        return Err(BotError::ShortTransfer);
    }
    let last_lba = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if last_lba == u32::MAX {
        return Err(BotError::Capacity16Required);
    }
    let block_size = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if block_size != BLOCK_SIZE as u32 {
        return Err(BotError::UnsupportedBlockSize(block_size));
    }
    Ok(Capacity {
        blocks: last_lba as u64 + 1,
        block_size,
    })
}

/// Hardware-facing adapter used by the controller.  Each call must be bounded
/// and must return `Stall` distinctly so BOT recovery can be selected.
pub trait BotTransport {
    fn control_in(&mut self, request: SetupRequest, data: &mut [u8]) -> Result<usize, BotError>;
    fn control_out(&mut self, request: SetupRequest) -> Result<(), BotError>;
    fn bulk_out(&mut self, data: &[u8]) -> Result<usize, BotError>;
    fn bulk_in(&mut self, data: &mut [u8]) -> Result<usize, BotError>;
}

pub struct Bot<'a, T> {
    transport: &'a mut T,
    interface: u8,
    bulk_in: u8,
    bulk_out: u8,
    next_tag: u32,
}

impl<'a, T: BotTransport> Bot<'a, T> {
    pub fn new(transport: &'a mut T, interface: u8, bulk_in: u8, bulk_out: u8) -> Self {
        Self {
            transport,
            interface,
            bulk_in,
            bulk_out,
            next_tag: 1,
        }
    }

    pub fn max_lun(&mut self) -> Result<u8, BotError> {
        let mut max_lun = [0u8; 1];
        match self
            .transport
            .control_in(get_max_lun(self.interface), &mut max_lun)
        {
            Ok(1) => Ok(max_lun[0]),
            Ok(_) => Err(BotError::ShortTransfer),
            Err(BotError::Stall) => Ok(0),
            Err(error) => Err(error),
        }
    }

    pub fn recover(&mut self) -> Result<(), BotError> {
        self.transport.control_out(bot_reset(self.interface))?;
        self.transport.control_out(clear_halt(self.bulk_in))?;
        self.transport.control_out(clear_halt(self.bulk_out))
    }

    pub fn command(
        &mut self,
        lun: u8,
        cdb: &[u8],
        direction: DataDirection,
        data: &mut [u8],
    ) -> Result<(), BotError> {
        // BOT recovery is deliberately bounded: a transport/protocol failure
        // gets one reset-and-retry, while a SCSI command failure is reported
        // to the caller without pretending the command succeeded.
        for attempt in 0..2 {
            let result = self.command_once(lun, cdb, direction, data);
            match result {
                Ok(()) => return Ok(()),
                Err(error)
                    if attempt == 0 && recovery_for(error) == Recovery::ResetAndClearHalts =>
                {
                    self.recover()?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(BotError::Transport)
    }

    fn command_once(
        &mut self,
        lun: u8,
        cdb: &[u8],
        direction: DataDirection,
        data: &mut [u8],
    ) -> Result<(), BotError> {
        if cdb.is_empty() || cdb.len() > 16 {
            return Err(BotError::Transport);
        }
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1);
        let packet = cbw(tag, data.len() as u32, direction, lun, cdb);
        if self.transport.bulk_out(&packet)? != CBW_LEN {
            return Err(BotError::ShortTransfer);
        }
        match direction {
            DataDirection::In if !data.is_empty() => {
                if self.transport.bulk_in(data)? != data.len() {
                    return Err(BotError::ShortTransfer);
                }
            }
            DataDirection::Out
                if !data.is_empty() && self.transport.bulk_out(data)? != data.len() =>
            {
                return Err(BotError::ShortTransfer);
            }
            _ => {}
        }
        let mut status = [0u8; CSW_LEN];
        if self.transport.bulk_in(&mut status)? != CSW_LEN {
            return Err(BotError::ShortTransfer);
        }
        Csw::parse(&status)?.validate(tag, data.len())
    }

    pub fn probe(&mut self) -> Result<Capacity, BotError> {
        let lun = self.max_lun()?;
        if lun != 0 {
            return Err(BotError::Transport);
        }
        let mut inquiry_data = [0u8; 36];
        self.command(0, &inquiry(), DataDirection::In, &mut inquiry_data)?;
        let mut ready = [];
        let mut ready_error = None;
        for _ in 0..3 {
            match self.command(0, &test_unit_ready(), DataDirection::None, &mut ready) {
                Ok(()) => {
                    ready_error = None;
                    break;
                }
                Err(error) => ready_error = Some(error),
            }
        }
        if let Some(error) = ready_error {
            // Sense data is diagnostic only; preserve the failed TUR result.
            let mut sense = [0u8; 18];
            let _ = self.command(0, &request_sense(), DataDirection::In, &mut sense);
            return Err(error);
        }
        let mut capacity = [0u8; 8];
        self.command(0, &read_capacity10(), DataDirection::In, &mut capacity)?;
        parse_capacity10(&capacity)
    }

    pub fn read10(&mut self, lba: u32, data: &mut [u8]) -> Result<(), BotError> {
        if data.is_empty() || !data.len().is_multiple_of(BLOCK_SIZE) {
            return Err(BotError::ReadTooLarge);
        }
        let mut current_lba = lba;
        let mut remaining = data.len();
        for chunk in data.chunks_mut(MAX_READ_BYTES) {
            let blocks =
                u16::try_from(chunk.len() / BLOCK_SIZE).map_err(|_| BotError::ReadTooLarge)?;
            self.command(0, &read10(current_lba, blocks), DataDirection::In, chunk)?;
            remaining -= chunk.len();
            if remaining != 0 {
                current_lba = current_lba
                    .checked_add(blocks as u32)
                    .ok_or(BotError::LbaOverflow)?;
            }
        }
        Ok(())
    }
}

pub fn test() {
    let packet = cbw(
        0x1122_3344,
        512,
        DataDirection::In,
        0,
        &read10(0x0102_0304, 1),
    );
    assert_eq!(packet.len(), CBW_LEN);
    assert_eq!(&packet[..4], b"USBC");
    assert_eq!(&packet[4..8], &[0x44, 0x33, 0x22, 0x11]);
    assert_eq!(packet[12], 0x80);
    assert_eq!(&packet[15..25], &[0x28, 0, 1, 2, 3, 4, 0, 0, 1, 0]);
    assert_eq!(read10(0x0102_0304, 2), [0x28, 0, 1, 2, 3, 4, 0, 0, 2, 0]);
    assert_eq!(
        parse_capacity10(&[0, 0, 0, 15, 0, 0, 2, 0]),
        Ok(Capacity {
            blocks: 16,
            block_size: 512
        })
    );
    assert_eq!(
        parse_capacity10(&[0xff; 8]),
        Err(BotError::Capacity16Required)
    );
    let valid = [
        0x55, 0x53, 0x42, 0x53, 0x44, 0x33, 0x22, 0x11, 0, 0, 0, 0, 0,
    ];
    assert_eq!(Csw::parse(&valid).unwrap().validate(0x1122_3344, 0), Ok(()));
    assert_eq!(
        Csw::parse(&valid).unwrap().validate(0, 0),
        Err(BotError::InvalidCswTag)
    );
    let residue = [
        0x55, 0x53, 0x42, 0x53, 0x44, 0x33, 0x22, 0x11, 1, 0, 0, 0, 0,
    ];
    assert_eq!(
        Csw::parse(&residue).unwrap().validate(0x1122_3344, 512),
        Err(BotError::InvalidResidue)
    );
    let failed = [
        0x55, 0x53, 0x42, 0x53, 0x44, 0x33, 0x22, 0x11, 0, 0, 0, 0, 1,
    ];
    assert_eq!(
        Csw::parse(&failed).unwrap().validate(0x1122_3344, 0),
        Err(BotError::CommandFailed)
    );
    assert_eq!(recovery_for(BotError::Stall), Recovery::ResetAndClearHalts);
    assert_eq!(recovery_for(BotError::CommandFailed), Recovery::Offline);
    assert_eq!(get_max_lun(3).index, 3);
    assert_eq!(bot_reset(3).request, 0xff);
}
