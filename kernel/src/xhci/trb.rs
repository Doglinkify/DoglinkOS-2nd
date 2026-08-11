//! xHCI TRB representation and constants.

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Trb {
    pub parameter: u64,
    pub status: u32,
    pub control: u32,
}

pub const TRB_CYCLE: u32 = 1;
pub const TRB_TC: u32 = 1 << 1;
pub const TRB_CHAIN: u32 = 1 << 4;
pub const TRB_TYPE_SHIFT: u32 = 10;
pub const TRB_TYPE_MASK: u32 = 0x3f << TRB_TYPE_SHIFT;
pub const TRB_TYPE_LINK: u32 = 6 << TRB_TYPE_SHIFT;
pub const TRB_TYPE_EVENT_DATA: u32 = 7 << TRB_TYPE_SHIFT;
pub const TRB_TYPE_COMMAND_COMPLETION: u32 = 33 << TRB_TYPE_SHIFT;
pub const TRB_TYPE_TRANSFER_EVENT: u32 = 32 << TRB_TYPE_SHIFT;
pub const TRB_TYPE_PORT_STATUS_CHANGE: u32 = 34 << TRB_TYPE_SHIFT;
pub const TRB_TYPE_SETUP_STAGE: u32 = 2 << TRB_TYPE_SHIFT;
pub const TRB_TYPE_NORMAL: u32 = 1 << TRB_TYPE_SHIFT;
pub const TRB_TYPE_DATA_STAGE: u32 = 3 << TRB_TYPE_SHIFT;
pub const TRB_TYPE_STATUS_STAGE: u32 = 4 << TRB_TYPE_SHIFT;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionCode {
    Invalid = 0,
    Success = 1,
    DataBufferError = 2,
    TrbError = 5,
    StallError = 6,
    ShortPacket = 13,
    RingUnderrun = 14,
    RingOverrun = 15,
    EventRingFull = 21,
    CommandAborted = 25,
    Stop = 26,
    ContextStateError = 19,
    NoSlotsAvailable = 9,
    Unknown = 255,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortStatusChangeEvent {
    pub port_id: u8,
}

/// Decode the xHCI Port Status Change Event TRB without touching MMIO.
pub fn port_status_change_event(event: Trb) -> Option<PortStatusChangeEvent> {
    if event.control & TRB_TYPE_MASK != TRB_TYPE_PORT_STATUS_CHANGE {
        return None;
    }
    let port_id = (event.parameter & 0xff) as u8;
    (port_id != 0).then_some(PortStatusChangeEvent { port_id })
}

impl CompletionCode {
    pub const fn from_status(status: u32) -> Self {
        match ((status >> 24) & 0xff) as u8 {
            0 => Self::Invalid,
            1 => Self::Success,
            2 => Self::DataBufferError,
            5 => Self::TrbError,
            6 => Self::StallError,
            9 => Self::NoSlotsAvailable,
            13 => Self::ShortPacket,
            14 => Self::RingUnderrun,
            15 => Self::RingOverrun,
            19 => Self::ContextStateError,
            21 => Self::EventRingFull,
            25 => Self::CommandAborted,
            26 => Self::Stop,
            _ => Self::Unknown,
        }
    }
}

pub fn link_trb(next: u64, cycle: bool) -> Trb {
    Trb {
        parameter: next,
        control: TRB_TYPE_LINK | TRB_TC | ((cycle as u32) * TRB_CYCLE),
        ..Trb::default()
    }
}

pub fn test() {
    assert_eq!(core::mem::size_of::<Trb>(), 16);
    assert_eq!(core::mem::align_of::<Trb>(), 16);
    let link = link_trb(0x1234_0000, true);
    assert_eq!(link.parameter, 0x1234_0000);
    assert_eq!(link.control & TRB_TYPE_MASK, TRB_TYPE_LINK);
    assert_ne!(link.control & TRB_TC, 0);
    assert_ne!(link.control & TRB_CYCLE, 0);
    assert_eq!(
        CompletionCode::from_status(1 << 24),
        CompletionCode::Success
    );
    assert_eq!(
        port_status_change_event(Trb {
            parameter: 7,
            control: TRB_TYPE_PORT_STATUS_CHANGE | TRB_CYCLE,
            ..Trb::default()
        }),
        Some(PortStatusChangeEvent { port_id: 7 })
    );
    assert_eq!(
        port_status_change_event(Trb {
            parameter: 0,
            control: TRB_TYPE_PORT_STATUS_CHANGE,
            ..Trb::default()
        }),
        None
    );
}
