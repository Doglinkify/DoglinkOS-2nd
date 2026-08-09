//! Small, allocation-free pieces of USB root-port enumeration.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupportedProtocol {
    pub major: u8,
    pub minor: u8,
    pub port_start: u8,
    pub port_count: u8,
    pub usb2: bool,
}

/// Decode a Supported Protocol extended capability.
/// The capability header carries the protocol revision; DWORD 2 carries the
/// compatible port range (DWORD 1 is the human-readable protocol name).
pub fn supported_protocol(dword0: u32, port_info: u32) -> Option<SupportedProtocol> {
    if dword0 & 0xff != 2 {
        return None;
    }
    let major = ((dword0 >> 24) & 0xff) as u8;
    let minor = ((dword0 >> 16) & 0xff) as u8;
    let port_start = (port_info & 0xff) as u8;
    let port_count = ((port_info >> 8) & 0xff) as u8;
    if port_start == 0 || port_count == 0 {
        return None;
    }
    Some(SupportedProtocol {
        major,
        minor,
        port_start,
        port_count,
        usb2: major == 2,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortState {
    Disconnected,
    Reset,
    Addressed {
        slot: u8,
        speed: u8,
        max_packet: u16,
    },
    Failed,
}

pub fn usb2_max_packet(speed: u8) -> u16 {
    match speed {
        3 => 64, // high speed
        _ => 8,  // low/full speed (the default control endpoint size)
    }
}

/// State transition used by the root-port scanner.  An addressed port is
/// terminal for this release, preventing repeated scans from allocating slots.
pub fn advance_port(
    state: PortState,
    connected: bool,
    reset_ok: bool,
    address_ok: bool,
    slot: u8,
    speed: u8,
) -> PortState {
    if matches!(state, PortState::Addressed { .. }) {
        return state;
    }
    if !connected {
        PortState::Disconnected
    } else if !reset_ok || !address_ok || slot == 0 {
        PortState::Failed
    } else {
        PortState::Addressed {
            slot,
            speed,
            max_packet: usb2_max_packet(speed),
        }
    }
}

pub fn test() {
    let cap = supported_protocol(2 | (2 << 24), 2 | (4 << 8)).unwrap();
    assert!(cap.usb2);
    assert_eq!(cap.port_start, 2);
    assert_eq!(cap.port_count, 4);
    assert!(supported_protocol(1 | (3 << 24), 1 | (1 << 8)).is_none());
    assert!(supported_protocol(2 | (2 << 24), 0).is_none());
    assert_eq!(usb2_max_packet(3), 64);
    assert_eq!(usb2_max_packet(2), 8);
    let addressed = PortState::Addressed {
        slot: 1,
        speed: 3,
        max_packet: 64,
    };
    assert_eq!(
        advance_port(PortState::Disconnected, true, true, true, 1, 3),
        addressed
    );
    assert_eq!(
        advance_port(PortState::Disconnected, true, false, true, 1, 3),
        PortState::Failed
    );
    assert_eq!(
        advance_port(PortState::Reset, true, true, false, 0, 3),
        PortState::Failed
    );
    assert_eq!(advance_port(addressed, true, true, true, 2, 3), addressed);
}
