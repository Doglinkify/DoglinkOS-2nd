fn compute_checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;

    while i + 1 < data.len() {
        let word = u16::from_be_bytes([data[i], data[i + 1]]);
        sum += word as u32;
        i += 2;
    }

    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }

    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    !(sum as u16)
}

pub fn try_handle_ping(packet: &[u8], payload_start: usize, handle: usize) {
    if packet.len() < payload_start + 8 {
        return;
    }

    let version_ihl = packet[0];
    let version = version_ihl >> 4;
    if version != 4 {
        return;
    }

    let protocol = packet[9];
    if protocol != 1 {
        return;
    }

    let src_ip = &packet[12..16];
    let dst_ip = &packet[16..20];

    let icmp_type = packet[payload_start];
    let icmp_code = packet[payload_start + 1];
    if icmp_type != 8 || icmp_code != 0 {
        return;
    }

    let mut reply = [0u8; 1500];
    let packet_len = packet.len();
    if packet_len > reply.len() {
        return;
    }
    reply[..packet_len].copy_from_slice(packet);

    reply[12..16].copy_from_slice(dst_ip);
    reply[16..20].copy_from_slice(src_ip);

    reply[payload_start] = 0;

    let icmp_checksum_off = payload_start + 2;
    reply[icmp_checksum_off..icmp_checksum_off + 2].copy_from_slice(&[0, 0]);
    let icmp_checksum = compute_checksum(&reply[payload_start..packet_len]);
    reply[icmp_checksum_off..icmp_checksum_off + 2].copy_from_slice(&icmp_checksum.to_be_bytes());

    let ip_checksum_off = 10;
    reply[ip_checksum_off..ip_checksum_off + 2].copy_from_slice(&[0, 0]);
    let ip_checksum = compute_checksum(&reply[..payload_start]);
    reply[ip_checksum_off..ip_checksum_off + 2].copy_from_slice(&ip_checksum.to_be_bytes());

    crate::netdump::send_ipv4(&reply[..packet_len], handle);
}
