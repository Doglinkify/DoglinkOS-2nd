//! Small, allocation-free pieces of USB root-port enumeration.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupportedProtocol {
    pub major: u8,
    pub minor: u8,
    pub port_start: u8,
    pub port_count: u8,
    pub usb2: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootPortState {
    Disconnected,
    Debouncing { until: u64 },
    Resetting,
    Enumerating,
    Active,
    Removing,
    Failed { retry_at: u64, failures: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortRecord {
    pub port: u8,
    pub protocol: SupportedProtocol,
    pub portsc: u32,
    pub generation: u64,
    pub state: RootPortState,
}

impl PortRecord {
    pub const DEBOUNCE_TICKS: u64 = 8;
    pub const RETRY_BASE_TICKS: u64 = 64;
    pub const RETRY_MAX_TICKS: u64 = 4096;

    pub const fn new(port: u8, protocol: SupportedProtocol, portsc: u32) -> Self {
        Self {
            port,
            protocol,
            portsc,
            generation: 0,
            state: RootPortState::Disconnected,
        }
    }

    pub fn acknowledge_changes(&mut self, portsc: u32) -> u32 {
        let changes = portsc & crate::xhci::regs::PORTSC_CHANGE_MASK;
        self.portsc = portsc & !crate::xhci::regs::PORTSC_CHANGE_MASK;
        changes
    }

    pub fn observe(&mut self, portsc: u32, now: u64) -> u32 {
        let was_connected = self.portsc & crate::xhci::regs::PORTSC_CCS != 0;
        let changes = self.acknowledge_changes(portsc);
        let connected = portsc & crate::xhci::regs::PORTSC_CCS != 0;
        if changes != 0 || connected != was_connected {
            self.generation = self.generation.wrapping_add(1);
            self.state = if connected {
                RootPortState::Debouncing {
                    until: now.saturating_add(Self::DEBOUNCE_TICKS),
                }
            } else {
                RootPortState::Removing
            };
        }
        self.portsc = portsc & !crate::xhci::regs::PORTSC_CHANGE_MASK;
        changes
    }

    pub fn mark_active(&mut self) {
        self.state = RootPortState::Active;
    }
    pub fn mark_stage(&mut self, state: RootPortState) {
        self.state = state;
    }
    pub fn generation_matches(&self, generation: u64) -> bool {
        self.generation == generation
    }

    pub fn mark_failed(&mut self, now: u64) {
        let failures = match self.state {
            RootPortState::Failed { failures, .. } => failures.saturating_add(1),
            _ => 1,
        };
        let shift = failures.saturating_sub(1).min(6) as u32;
        let delay = (Self::RETRY_BASE_TICKS << shift).min(Self::RETRY_MAX_TICKS);
        self.state = RootPortState::Failed {
            retry_at: now.saturating_add(delay),
            failures,
        };
    }

    pub fn retry_due(&self, now: u64) -> bool {
        matches!(self.state, RootPortState::Failed { retry_at, .. } if now >= retry_at)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorError {
    Truncated,
    ZeroLength,
    InvalidLength,
    MissingDevice,
    MissingConfiguration,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HidBootEndpoint {
    pub configuration_value: u8,
    pub interface_number: u8,
    pub endpoint_address: u8,
    pub max_packet_size: u16,
    pub interval: u8,
    pub protocol: u8,
}

/// Select one HID Boot keyboard or mouse interface from a complete
/// configuration descriptor.  Unknown descriptors and alternate settings are
/// skipped, while malformed descriptor framing is rejected.
pub fn parse_hid_boot_endpoint(data: &[u8]) -> Result<HidBootEndpoint, DescriptorError> {
    let mut offset = 0usize;
    let mut configuration = None;
    let mut interface: Option<(u8, u8, u8)> = None;
    let mut selected = None;
    while offset < data.len() {
        if data.len() - offset < 2 {
            return Err(DescriptorError::Truncated);
        }
        let length = data[offset] as usize;
        if length == 0 {
            return Err(DescriptorError::ZeroLength);
        }
        if length < 2 {
            return Err(DescriptorError::InvalidLength);
        }
        let end = offset
            .checked_add(length)
            .ok_or(DescriptorError::Truncated)?;
        if end > data.len() {
            return Err(DescriptorError::Truncated);
        }
        let kind = data[offset + 1];
        match kind {
            2 => {
                if length < 9 {
                    return Err(DescriptorError::InvalidLength);
                }
                let total = u16::from_le_bytes([data[offset + 2], data[offset + 3]]) as usize;
                if total < length || total > data.len() {
                    return Err(DescriptorError::Truncated);
                }
                configuration = Some(data[offset + 5]);
            }
            4 => {
                if length < 9 {
                    return Err(DescriptorError::InvalidLength);
                }
                let number = data[offset + 2];
                let alternate = data[offset + 3];
                let class = data[offset + 5];
                let subclass = data[offset + 6];
                let protocol = data[offset + 7];
                interface = if alternate == 0
                    && class == 0x03
                    && subclass == 0x01
                    && (protocol == 0x01 || protocol == 0x02)
                {
                    Some((number, protocol, alternate))
                } else {
                    None
                };
            }
            5 => {
                if length < 7 {
                    // An endpoint descriptor is meaningful only inside a
                    // selected interface.  Reject a malformed one there;
                    // unrelated class-specific descriptors remain skippable.
                    if interface.is_some() {
                        return Err(DescriptorError::InvalidLength);
                    }
                    offset = end;
                    continue;
                }
                if let Some((number, protocol, 0)) = interface {
                    let address = data[offset + 2];
                    let attributes = data[offset + 3] & 0x03;
                    let max_packet =
                        u16::from_le_bytes([data[offset + 4], data[offset + 5]]) & 0x07ff;
                    if address & 0x80 != 0
                        && attributes == 0x03
                        && max_packet != 0
                        && data[offset + 6] != 0
                    {
                        if selected.is_some() {
                            return Err(DescriptorError::Unsupported);
                        }
                        selected = Some(HidBootEndpoint {
                            configuration_value: configuration.unwrap_or(0),
                            interface_number: number,
                            endpoint_address: address,
                            max_packet_size: max_packet,
                            interval: data[offset + 6],
                            protocol,
                        });
                    }
                }
            }
            _ => {}
        }
        offset = end;
    }
    let endpoint = selected.ok_or(DescriptorError::Unsupported)?;
    if endpoint.configuration_value == 0 {
        return Err(DescriptorError::MissingConfiguration);
    }
    // The interval is consumed by the xHCI endpoint context, where zero is
    // invalid for interrupt endpoints even though the descriptor framing is
    // otherwise valid.
    if endpoint.interval == 0 {
        return Err(DescriptorError::InvalidLength);
    }
    Ok(endpoint)
}

/// Compatibility name used by the controller enumeration state machine.
pub fn parse_configuration(data: &[u8]) -> Result<HidBootEndpoint, DescriptorError> {
    parse_hid_boot_endpoint(data)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetupRequest {
    pub bm_request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}

pub const fn get_descriptor(descriptor_type: u8, length: u16) -> SetupRequest {
    SetupRequest {
        bm_request_type: 0x80,
        request: 6,
        value: (descriptor_type as u16) << 8,
        index: 0,
        length,
    }
}

pub const fn set_configuration(value: u8) -> SetupRequest {
    SetupRequest {
        bm_request_type: 0,
        request: 9,
        value: value as u16,
        index: 0,
        length: 0,
    }
}

pub const fn set_idle(interface: u8) -> SetupRequest {
    SetupRequest {
        bm_request_type: 0x21,
        request: 10,
        value: 0,
        index: interface as u16,
        length: 0,
    }
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
    let config = [
        9, 2, 25, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 3, 1, 1, 0, 7, 5, 0x81, 3, 8, 0, 10,
    ];
    assert_eq!(parse_hid_boot_endpoint(&config).unwrap().interval, 10);
    assert_eq!(
        parse_hid_boot_endpoint(&[0, 2]).unwrap_err(),
        DescriptorError::ZeroLength
    );
    assert_eq!(
        parse_hid_boot_endpoint(&[9, 2, 0]).unwrap_err(),
        DescriptorError::Truncated
    );
    assert_eq!(
        parse_hid_boot_endpoint(&[9, 2, 13, 0, 1, 1, 0, 0x80, 50, 4, 4, 0, 0]).unwrap_err(),
        DescriptorError::InvalidLength
    );
    assert_eq!(
        parse_hid_boot_endpoint(&[
            9, 2, 25, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 3, 1, 1, 0, 5, 5, 0x81, 3, 8, 0,
        ])
        .unwrap_err(),
        DescriptorError::Truncated
    );
    assert_eq!(
        parse_hid_boot_endpoint(&[9, 2, 18, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 0, 3, 1, 1, 0,])
            .unwrap_err(),
        DescriptorError::Unsupported
    );
    assert_eq!(get_descriptor(2, 9).value, 2 << 8);
    assert_eq!(set_configuration(3).value, 3);
    let protocol = SupportedProtocol {
        major: 2,
        minor: 0,
        port_start: 1,
        port_count: 1,
        usb2: true,
    };
    let mut record = PortRecord::new(1, protocol, 0);
    assert_eq!(
        record.observe(
            crate::xhci::regs::PORTSC_CCS | crate::xhci::regs::PORTSC_CSC,
            10
        ),
        crate::xhci::regs::PORTSC_CSC
    );
    assert!(matches!(
        record.state,
        RootPortState::Debouncing { until: 18 }
    ));
    let generation = record.generation;
    assert!(record.generation_matches(generation));
    assert!(!record.generation_matches(generation.wrapping_sub(1)));
    record.mark_failed(20);
    assert!(!record.retry_due(20));
    assert!(record.retry_due(84));
    record.mark_failed(84);
    assert!(!record.retry_due(148));
    assert!(record.retry_due(212));
    record.observe(crate::xhci::regs::PORTSC_PEC, 90);
    assert_eq!(
        record.acknowledge_changes(crate::xhci::regs::PORTSC_PEC),
        crate::xhci::regs::PORTSC_PEC
    );
    record.mark_active();
    record.observe(crate::xhci::regs::PORTSC_CSC, 100);
    assert_eq!(record.state, RootPortState::Removing);
}
