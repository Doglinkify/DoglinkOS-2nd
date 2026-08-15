//! Checked, single-TD bulk transfer building blocks.
//!
//! BOT submits one command at a time.  Keeping this state separate from the
//! HID interrupt ring makes ownership and completion matching explicit.

use super::trb::{CompletionCode, TRB_CHAIN, TRB_CYCLE, TRB_TYPE_NORMAL, Trb};

pub const MAX_TD_TRANSFER: usize = 0x1_0000;
const TRB_IOC: u32 = 1 << 5;
const TRB_DIR_IN: u32 = 1 << 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BulkError {
    Empty,
    TooLarge,
    Completion(CompletionCode),
    InvalidResidual,
    ShortPacket,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BulkCompletion {
    pub transferred: usize,
    pub short_packet: bool,
}

/// Build the sole Normal TRB in a bulk TD.  IOC is always on this TRB, so the
/// event pointer identifies exactly one in-flight TD.
pub fn normal_trb(
    buffer: u64,
    len: usize,
    in_direction: bool,
    cycle: bool,
) -> Result<Trb, BulkError> {
    if len == 0 {
        return Err(BulkError::Empty);
    }
    if len > MAX_TD_TRANSFER {
        return Err(BulkError::TooLarge);
    }
    Ok(Trb {
        parameter: buffer,
        status: len as u32,
        control: TRB_TYPE_NORMAL
            | TRB_IOC
            | if in_direction { TRB_DIR_IN } else { 0 }
            | if cycle { TRB_CYCLE } else { 0 },
    })
}

/// Validate a single-TD completion.  OUT must complete in full; IN accepts a
/// short packet and reports the actual byte count to the class driver.
pub fn completion(
    code: CompletionCode,
    requested: usize,
    residual: usize,
    in_direction: bool,
) -> Result<BulkCompletion, BulkError> {
    if residual > requested {
        return Err(BulkError::InvalidResidual);
    }
    match code {
        CompletionCode::Success if residual == 0 => Ok(BulkCompletion {
            transferred: requested,
            short_packet: false,
        }),
        CompletionCode::Success | CompletionCode::ShortPacket if in_direction => {
            Ok(BulkCompletion {
                transferred: requested - residual,
                short_packet: residual != 0 || code == CompletionCode::ShortPacket,
            })
        }
        CompletionCode::ShortPacket => Err(BulkError::ShortPacket),
        other => Err(BulkError::Completion(other)),
    }
}

pub fn test() {
    let trb = normal_trb(0x2000, 512, true, true).unwrap();
    assert_eq!(trb.parameter, 0x2000);
    assert_eq!(trb.status, 512);
    assert_ne!(trb.control & TRB_IOC, 0);
    assert_ne!(trb.control & TRB_DIR_IN, 0);
    assert_eq!(trb.control & TRB_CHAIN, 0);
    assert_eq!(normal_trb(0, 0, false, true), Err(BulkError::Empty));
    assert_eq!(
        normal_trb(0, MAX_TD_TRANSFER + 1, false, true),
        Err(BulkError::TooLarge)
    );
    assert_eq!(
        completion(CompletionCode::ShortPacket, 512, 64, true),
        Ok(BulkCompletion {
            transferred: 448,
            short_packet: true
        })
    );
    assert_eq!(
        completion(CompletionCode::ShortPacket, 512, 64, false),
        Err(BulkError::ShortPacket)
    );
    assert_eq!(
        completion(CompletionCode::Success, 512, 513, true),
        Err(BulkError::InvalidResidual)
    );
}
