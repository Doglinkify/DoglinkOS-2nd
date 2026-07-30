#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use dlos_app_rt::*;
use good_memory_allocator::SpinLockedAllocator;
use ppproto::pppos::{PPPoS, PPPoSAction};
use ppproto::{Config, Phase};

#[global_allocator]
static ALLOCATOR: SpinLockedAllocator = SpinLockedAllocator::empty();

const SERIAL_PATH: &str = "/dev/serial";
const SERVICE_NAME: &str = "upppd";

const IPC_EAGAIN: isize = -11;

const OP_SEND_IPV4: u8 = 1;
const OP_QUERY_STATUS: u8 = 2;

const EVT_ACK: u8 = 0x80;
const EVT_RX_IPV4: u8 = 0x81;
const EVT_STATUS: u8 = 0x82;
const EVT_ERROR: u8 = 0xFF;

const SERIAL_RX_BUF_SIZE: usize = 2048;
const PPP_TX_BUF_SIZE: usize = 2304;
const IPC_BUF_SIZE: usize = 4096;

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    init_heap();
    main();
    sys_exit();
}

fn init_heap() {
    unsafe {
        let old_brk: usize;
        core::arch::asm!(
            "int 0x80",
            in("rax") 7,
            in("rdi") 0,
            out("rsi") old_brk,
        );
        core::arch::asm!(
            "int 0x80",
            in("rax") 7,
            in("rdi") old_brk + (1 << 23),
            out("rsi") _,
        );
        ALLOCATOR.init(old_brk, 1 << 23);
    }
}

fn main() {
    let Some(serial_fd) = sys_open(SERIAL_PATH, false) else {
        eprintln!("upppd: open {SERIAL_PATH} failed");
        return;
    };
    let Some(listener) = sys_ipc_bind(SERVICE_NAME) else {
        eprintln!("upppd: bind {SERVICE_NAME} failed");
        sys_close(serial_fd);
        return;
    };

    let mut ppp = PPPoS::new(Config {
        username: b"",
        password: b"",
    });
    if ppp.open().is_err() {
        eprintln!("upppd: PPP open failed");
        let _ = sys_ipc_close(listener);
        sys_close(serial_fd);
        return;
    }

    let mut clients: Vec<usize> = Vec::new();
    let mut serial_in = [0u8; 256];
    let mut serial_rx_buf = [0u8; SERIAL_RX_BUF_SIZE];
    let mut tx_buf = [0u8; PPP_TX_BUF_SIZE];
    let mut ipc_buf = [0u8; IPC_BUF_SIZE];
    let mut status_phase = Phase::Dead;

    loop {
        while let Some(handle) = sys_ipc_accept(listener) {
            let status = build_status_event(ppp.status().phase, ppp.status().ipv4.as_ref());
            let _ = sys_ipc_send(handle, &status);
            clients.push(handle);
        }

        let read_len = sys_read3(serial_fd, &mut serial_in);
        if read_len != 0 {
            let mut consumed = 0;
            while consumed < read_len {
                let used = ppp.consume(&serial_in[consumed..read_len], &mut serial_rx_buf);
                if used == 0 {
                    let mut progressed = false;
                    while handle_ppp_actions(
                        &mut ppp,
                        serial_fd,
                        &mut tx_buf,
                        &mut serial_rx_buf,
                        &mut clients,
                    ) {
                        progressed = true;
                    }
                    if !progressed {
                        break;
                    }
                    continue;
                }
                consumed += used;
            }
        }

        while handle_ppp_actions(
            &mut ppp,
            serial_fd,
            &mut tx_buf,
            &mut serial_rx_buf,
            &mut clients,
        ) {}

        let current_status = ppp.status();
        if current_status.phase != status_phase {
            status_phase = current_status.phase;
            let event = build_status_event(current_status.phase, current_status.ipv4.as_ref());
            broadcast(&mut clients, &event);
        }

        let mut idx = 0;
        while idx < clients.len() {
            let handle = clients[idx];
            let recv_len = sys_ipc_recv(handle, &mut ipc_buf);
            if recv_len == IPC_EAGAIN {
                idx += 1;
                continue;
            }
            if recv_len <= 0 {
                let _ = sys_ipc_close(handle);
                clients.swap_remove(idx);
                continue;
            }

            let reply =
                process_client_request(&mut ppp, &ipc_buf[..recv_len as usize], &mut tx_buf);
            match reply {
                ClientReply::Immediate(buf) => {
                    if sys_ipc_send(handle, &buf) < 0 {
                        let _ = sys_ipc_close(handle);
                        clients.swap_remove(idx);
                        continue;
                    }
                }
                ClientReply::Transmit(len) => {
                    write_raw(serial_fd, &tx_buf[..len]);
                    let ack = build_ack(0);
                    if sys_ipc_send(handle, &ack) < 0 {
                        let _ = sys_ipc_close(handle);
                        clients.swap_remove(idx);
                        continue;
                    }
                }
            }
            idx += 1;
        }

        core::hint::spin_loop();
    }
}

enum ClientReply {
    Immediate([u8; 32]),
    Transmit(usize),
}

fn process_client_request(ppp: &mut PPPoS<'_>, req: &[u8], tx_buf: &mut [u8]) -> ClientReply {
    if req.is_empty() {
        return ClientReply::Immediate(build_error(1));
    }
    match req[0] {
        OP_SEND_IPV4 => {
            let payload = &req[1..];
            match ppp.send(payload, tx_buf) {
                Ok(len) => ClientReply::Transmit(len),
                Err(_) => ClientReply::Immediate(build_error(2)),
            }
        }
        OP_QUERY_STATUS => {
            let status = ppp.status();
            ClientReply::Immediate(build_status_event(status.phase, status.ipv4.as_ref()))
        }
        _ => ClientReply::Immediate(build_error(3)),
    }
}

fn handle_ppp_actions(
    ppp: &mut PPPoS<'_>,
    serial_fd: usize,
    tx_buf: &mut [u8],
    rx_buf: &mut [u8],
    clients: &mut Vec<usize>,
) -> bool {
    match ppp.poll(tx_buf, rx_buf) {
        PPPoSAction::None => false,
        PPPoSAction::Transmit(len) => {
            write_raw(serial_fd, &tx_buf[..len]);
            true
        }
        PPPoSAction::Received(range) => {
            let mut out = Vec::with_capacity(1 + range.len());
            out.push(EVT_RX_IPV4);
            out.extend_from_slice(&rx_buf[range]);
            broadcast(clients, &out);
            true
        }
    }
}

fn broadcast(clients: &mut Vec<usize>, payload: &[u8]) {
    let mut idx = 0;
    while idx < clients.len() {
        let handle = clients[idx];
        if sys_ipc_send(handle, payload) < 0 {
            let _ = sys_ipc_close(handle);
            clients.swap_remove(idx);
        } else {
            idx += 1;
        }
    }
}

fn build_ack(code: u8) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[0] = EVT_ACK;
    buf[1] = code;
    buf
}

fn build_error(code: u8) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[0] = EVT_ERROR;
    buf[1] = code;
    buf
}

fn build_status_event(
    phase: Phase,
    ipv4: Option<&ppproto::Ipv4Status>,
) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[0] = EVT_STATUS;
    buf[1] = match phase {
        Phase::Dead => 0,
        Phase::Establish => 1,
        Phase::Auth => 2,
        Phase::Network => 3,
        Phase::Open => 4,
    };
    if let Some(ipv4) = ipv4 {
        if let Some(address) = ipv4.address {
            buf[2..6].copy_from_slice(&address.octets());
        }
        if let Some(peer) = ipv4.peer_address {
            buf[6..10].copy_from_slice(&peer.octets());
        }
        if let Some(dns1) = ipv4.dns_servers[0] {
            buf[10..14].copy_from_slice(&dns1.octets());
        }
        if let Some(dns2) = ipv4.dns_servers[1] {
            buf[14..18].copy_from_slice(&dns2.octets());
        }
    }
    buf
}

fn write_raw(fd: usize, buf: &[u8]) {
    let raw = unsafe { core::str::from_utf8_unchecked(buf) };
    sys_write(fd, raw);
}
