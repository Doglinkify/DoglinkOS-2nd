use dlos_app_rt::*;

const IPC_EAGAIN: isize = -11;

fn recv_poll(handle: usize, buf: &mut [u8]) -> isize {
    loop {
        let recv_len = sys_ipc_recv(handle, buf);
        if recv_len != IPC_EAGAIN {
            return recv_len;
        }
        core::hint::spin_loop();
    }
}

fn connect_poll(name: &str) -> usize {
    loop {
        if let Some(handle) = sys_ipc_connect(name) {
            return handle;
        }
        core::hint::spin_loop();
    }
}

pub fn main(mut cnt: usize) {
    let handle = connect_poll("upppd");
    let mut buf = [0u8; 4096];
    loop {
        let msg_len = recv_poll(handle, &mut buf) as usize;
        let msg = &mut buf[..msg_len];
        match msg[0] {
            0x81 => handle_inbound_ipv4(&msg[1..], handle),
            0x82 => handle_status(&msg),
            _ => continue,
        }
        cnt -= 1;
        if cnt == 0 {
            break;
        }
    }
}

fn print_ipv4_header(packet: &[u8]) -> Option<usize> {
    if packet.len() < 20 {
        println!("Error: packet too short for IPv4 header");
        return None;
    }

    let version_ihl = packet[0];
    let version = version_ihl >> 4;
    let ihl = version_ihl & 0x0F;

    if version != 4 {
        println!("Error: not IPv4 (version={})", version);
        return None;
    }
    if ihl < 5 {
        println!("Error: invalid IHL ({})", ihl);
        return None;
    }

    let header_len = (ihl as usize) * 4;
    if packet.len() < header_len {
        println!("Error: packet length less than header length");
        return None;
    }

    // Version & IHL
    println!("Version: {}", version);
    println!("IHL: {} ({} bytes)", ihl, header_len);

    // DSCP & ECN
    let dscp_ecn = packet[1];
    println!("DSCP: {}", dscp_ecn >> 2);
    println!("ECN: {}", dscp_ecn & 0x03);

    // Total Length
    let total_len = u16::from_be_bytes([packet[2], packet[3]]);
    println!("Total Length: {}", total_len);

    // Identification
    let id = u16::from_be_bytes([packet[4], packet[5]]);
    println!("Identification: {}", id);

    // Flags & Fragment Offset
    let flags_frag = u16::from_be_bytes([packet[6], packet[7]]);
    let flags = (flags_frag >> 13) as u8;
    let frag_off = flags_frag & 0x1FFF;
    println!(
        "Flags: reserved={}, DF={}, MF={}",
        (flags >> 2) & 1,
        (flags >> 1) & 1,
        flags & 1
    );
    println!("Fragment Offset: {}", frag_off);

    // TTL
    println!("TTL: {}", packet[8]);

    // Protocol
    println!("Protocol: {}", packet[9]);

    // Header Checksum
    let checksum = u16::from_be_bytes([packet[10], packet[11]]);
    println!("Header Checksum: 0x{:04X}", checksum);

    // Source Address
    print!("Source Address: ");
    print_ipv4_addr(&packet[12..16]);
    println!();

    // Destination Address
    print!("Destination Address: ");
    print_ipv4_addr(&packet[16..20]);
    println!();

    // Options (if any)
    let opt_len = header_len - 20;
    if opt_len > 0 {
        print!("Options ({} bytes): ", opt_len);
        for i in 0..opt_len {
            print!("{:02X} ", packet[20 + i]);
        }
        println!();
    } else {
        println!("Options: (none)");
    }

    let payload_start = header_len;
    if packet.len() > payload_start {
        let payload_len = core::cmp::min(packet.len() - payload_start, 64);
        print!("Payload (first {} bytes): ", payload_len);
        for i in 0..payload_len {
            print!("{:02X} ", packet[payload_start + i]);
        }
        println!();
    } else {
        println!("Payload: (none)");
    }

    Some(payload_start)
}

fn print_ipv4_addr(addr: &[u8]) {
    if addr.len() >= 4 {
        print!("{}.{}.{}.{}", addr[0], addr[1], addr[2], addr[3]);
    } else {
        print!("<invalid>");
    }
}

pub fn send_ipv4(packet: &[u8], handle: usize) {
    let mut buf = [0u8; 4096];
    buf[0] = 1;
    buf[1..(packet.len() + 1)].copy_from_slice(packet);
    sys_ipc_send(handle, &buf[..packet.len() + 1]);
}

fn handle_inbound_ipv4(packet: &[u8], handle: usize) {
    println!("---------- BEGIN IPV4 PACKET ----------");
    if let Some(payload_start) = print_ipv4_header(packet) {
        crate::icmp::try_handle_ping(packet, payload_start, handle);
    }
    println!("----------- END IPV4 PACKET -----------");
}

fn handle_status(status: &[u8]) {
    let phase = status[1];
    let local_addr = core::net::Ipv4Addr::new(status[2], status[3], status[4], status[5]);
    let peer_addr = core::net::Ipv4Addr::new(status[6], status[7], status[8], status[9]);
    let dns1 = core::net::Ipv4Addr::new(status[10], status[11], status[12], status[13]);
    let dns2 = core::net::Ipv4Addr::new(status[14], status[15], status[16], status[17]);
    println!("--------- BEGIN STATUS REPORT ---------");
    println!("Phase: {phase}");
    println!("Local Address: {local_addr}");
    println!("Peer Address: {peer_addr}");
    println!("DNS 1: {dns1}");
    println!("DNS 2: {dns2}");
    println!("---------- END STATUS REPORT ----------");
}
