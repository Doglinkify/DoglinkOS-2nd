#![no_std]
#![no_main]

use dlos_app_rt::*;

const IPC_FLAG_NONBLOCK: usize = 1;
const IPC_EAGAIN: isize = -11;
const NAMED_CHANNEL: &str = "ipc-demo.named";

fn recv_poll(handle: usize, buf: &mut [u8]) -> isize {
    loop {
        let recv_len = sys_ipc_recv(handle, buf, IPC_FLAG_NONBLOCK);
        if recv_len != IPC_EAGAIN {
            return recv_len;
        }
        core::hint::spin_loop();
    }
}

fn connect_blocking(name: &str) -> usize {
    loop {
        if let Some(handle) = sys_ipc_connect(name) {
            return handle;
        }
        core::hint::spin_loop();
    }
}

fn accept_blocking(listener: usize) -> usize {
    loop {
        if let Some(handle) = sys_ipc_accept(listener) {
            return handle;
        }
        core::hint::spin_loop();
    }
}

fn run_named_client(client_id: usize) -> ! {
    let handle = connect_blocking(NAMED_CHANNEL);
    println!("ipc-demo client {client_id} connected");

    let mut outbound = [0u8; 32];
    let outbound_len = {
        let msg = b"hello from client ";
        outbound[..msg.len()].copy_from_slice(msg);
        outbound[msg.len()] = b'0' + client_id as u8;
        msg.len() + 1
    };
    let send_len = sys_ipc_send(handle, &outbound[..outbound_len], 0);
    if send_len < 0 {
        eprintln!("ipc-demo client {client_id}: send failed {}", send_len);
    }

    let mut buf = [0u8; 128];
    let recv_len = recv_poll(handle, &mut buf);
    if recv_len < 0 {
        eprintln!("ipc-demo client {client_id}: recv failed {}", recv_len);
    } else {
        let msg = core::str::from_utf8(&buf[..recv_len as usize]).unwrap_or("<invalid utf8>");
        println!("ipc-demo client {client_id} received: {msg}");
    }

    let _ = sys_ipc_close(handle);
    sys_exit();
}

fn run_named_server() -> ! {
    let Some(listener) = sys_ipc_bind(NAMED_CHANNEL) else {
        eprintln!("ipc-demo server: bind failed");
        sys_exit();
    };
    println!("ipc-demo server listening on {NAMED_CHANNEL}");

    for client_id in 1..=3 {
        let conn = accept_blocking(listener);
        let mut buf = [0u8; 128];
        let recv_len = recv_poll(conn, &mut buf);
        if recv_len < 0 {
            eprintln!(
                "ipc-demo server: recv from client {client_id} failed {}",
                recv_len
            );
        } else {
            let msg = core::str::from_utf8(&buf[..recv_len as usize]).unwrap_or("<invalid utf8>");
            println!("ipc-demo server accepted client {client_id}: {msg}");

            let mut reply = [0u8; 32];
            let prefix = b"ack ";
            reply[..prefix.len()].copy_from_slice(prefix);
            reply[prefix.len()] = b'0' + client_id as u8;
            let reply_len = prefix.len() + 1;
            let send_len = sys_ipc_send(conn, &reply[..reply_len], 0);
            if send_len < 0 {
                eprintln!(
                    "ipc-demo server: reply to client {client_id} failed {}",
                    send_len
                );
            }
        }
        let _ = sys_ipc_close(conn);
    }

    let _ = sys_ipc_close(listener);
    sys_exit();
}

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    let Some((parent_end, child_end)) = sys_ipc_create(0) else {
        eprintln!("ipc-demo: ipc_create failed");
        sys_exit();
    };

    let pid = sys_fork();
    if pid == 0 {
        let _ = sys_ipc_close(parent_end);

        let mut buf = [0u8; 128];
        let recv_len = recv_poll(child_end, &mut buf);
        if recv_len < 0 {
            eprintln!("ipc-demo child: recv failed {}", recv_len);
        } else {
            let msg = core::str::from_utf8(&buf[..recv_len as usize]).unwrap_or("<invalid utf8>");
            println!("ipc-demo child received: {msg}");

            let reply = b"hello from child";
            let send_len = sys_ipc_send(child_end, reply, 0);
            if send_len < 0 {
                eprintln!("ipc-demo child: send failed {}", send_len);
            } else {
                println!("ipc-demo child sent {} bytes", send_len);
            }
        }

        let _ = sys_ipc_close(child_end);
        sys_exit();
    }

    let _ = sys_ipc_close(child_end);

    let msg = b"hello from parent";
    let send_len = sys_ipc_send(parent_end, msg, 0);
    if send_len < 0 {
        eprintln!("ipc-demo parent: send failed {}", send_len);
    } else {
        println!("ipc-demo parent sent {} bytes", send_len);
    }

    let mut buf = [0u8; 128];
    let recv_len = recv_poll(parent_end, &mut buf);
    if recv_len < 0 {
        eprintln!("ipc-demo parent: recv failed {}", recv_len);
    } else {
        let msg = core::str::from_utf8(&buf[..recv_len as usize]).unwrap_or("<invalid utf8>");
        println!("ipc-demo parent received: {msg}");
    }

    let _ = sys_ipc_close(parent_end);
    sys_waitpid(pid);

    let server_pid = sys_fork();
    if server_pid == 0 {
        run_named_server();
    }

    let mut client_pids = [0usize; 3];
    for (idx, slot) in client_pids.iter_mut().enumerate() {
        let fork_pid = sys_fork();
        if fork_pid == 0 {
            run_named_client(idx + 1);
        }
        *slot = fork_pid;
    }

    for pid in client_pids {
        sys_waitpid(pid);
    }
    sys_waitpid(server_pid);
    sys_exit();
}
