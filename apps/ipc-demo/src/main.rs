#![no_std]
#![no_main]

use dlos_app_rt::*;

const IPC_FLAG_NONBLOCK: usize = 1;
const IPC_EAGAIN: isize = -11;

fn recv_poll(handle: usize, buf: &mut [u8]) -> isize {
    loop {
        let recv_len = sys_ipc_recv(handle, buf, IPC_FLAG_NONBLOCK);
        if recv_len != IPC_EAGAIN {
            return recv_len;
        }
        core::hint::spin_loop();
    }
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
        buf[0] = 1; // trigger CoW on stack
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
    sys_exit();
}
