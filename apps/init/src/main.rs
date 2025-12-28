#![no_std]
#![no_main]

use dlos_app_rt::*;

struct Globals {
    pub t: vcell::VolatileCell<i32>,
}

unsafe impl Send for Globals {}
unsafe impl Sync for Globals {}

static TEST: Globals = Globals {
    t: vcell::VolatileCell::new(0),
};

fn read_line(buf: &mut [u8]) -> usize {
    for (i, v) in buf.iter_mut().enumerate() {
        match dlos_app_rt::sys_read() {
            b'\n' => return i,
            c => *v = c,
        }
    }
    buf.len()
}

fn shell_main_loop() {
    let mut buf = [0u8; 128];
    loop {
        print!("[User@DoglinkOS-2nd /]$ ");
        let len = read_line(&mut buf);
        let cmd = str::from_utf8(&buf[..len]).unwrap();
        if cmd.is_empty() {
            continue;
        }
        if cmd == "panic-test" {
            panic!("panic test");
        } else if cmd == "exit" {
            break;
        } else if cmd == "sysinfo" {
            println!("DoglinkOS-2nd version 1.3 Snapshot 1228");
            println!("DoglinkOS Shell version 1.3 Snapshot 1228");
            println!("In user mode");
            println!(
                "Console: {} rows, {} cols",
                sys_info(1).unwrap(),
                sys_info(0).unwrap()
            );
            println!("Current shell PID: {}", sys_info(2).unwrap());
            println!("Current kernel ticks: {}", sys_info(3).unwrap());
        } else if cmd.starts_with("echo ") {
            println!("{}", &cmd[5..]);
        } else if cmd == "clear" {
            print!("\x1b[H\x1b[2J\x1b[3J");
        } else if cmd == "file-read" {
            if let Some(fd) = sys_open("/test.txt", true) {
                let size = sys_seek(fd, 0, SEEK_END);
                println!("File /test.txt is {size} bytes");
                sys_seek(fd, 0, SEEK_SET);
                let mut content = [0; 512];
                sys_read2(fd, &mut content);
                println!("{:?}", &content[..size]);
                sys_close(fd);
            } else {
                println!("error while opening /test.txt");
            }
        } else if cmd == "disk-read" {
            if let Some(fd) = sys_open("/dev/disk0", false) {
                let mut content = [0; 512];
                sys_read2(fd, &mut content);
                println!("{content:?}");
                sys_close(fd);
            } else {
                println!("error while opening /dev/disk0");
            }
        } else if cmd == "nvme-read" {
            if let Some(fd) = sys_open("/dev/nvme0-0", false) {
                let mut content = [0; 512];
                sys_read2(fd, &mut content);
                println!("{content:?}");
                sys_close(fd);
            } else {
                println!("error while opening /dev/nvme0-0");
            }
        } else if cmd == "disk-size" {
            if let Some(fd) = sys_open("/dev/disk0", false) {
                let sz = sys_seek(fd, 0, SEEK_END);
                println!("/dev/disk0 is {sz:?} bytes");
                sys_close(fd);
            } else {
                println!("error while opening /dev/disk0");
            }
        } else if cmd == "nvme-size" {
            if let Some(fd) = sys_open("/dev/nvme0-0", false) {
                let sz = sys_seek(fd, 0, SEEK_END);
                println!("/dev/nvme0-0 is {sz:?} bytes");
                sys_close(fd);
            } else {
                println!("error while opening /dev/nvme0-0");
            }
        } else if cmd == "initrd-read" {
            if let Some(fd) = sys_open("/dev/initrd", false) {
                let mut content = [0; 512];
                sys_read2(fd, &mut content);
                println!("{content:?}");
                sys_close(fd);
            } else {
                println!("error while opening /dev/initrd");
            }
        } else if cmd.starts_with("file-write ") {
            if let Some(fd) = sys_open("/test.txt", true) {
                sys_write(fd, &cmd[11..]);
                sys_close(fd);
            } else {
                println!("error while opening /test.txt");
            }
        } else if cmd.starts_with("file-rm") {
            sys_remove("/test.txt");
        } else {
            let mut buf2 = [0u8; 128];
            buf2[0..5].copy_from_slice(b"/bin/");
            buf2[5..(5 + len)].copy_from_slice(&buf[..len]);
            let fork_result = sys_fork();
            if fork_result == 0 {
                sys_exec(unsafe { core::str::from_utf8_unchecked(&buf2[..(len + 5)]) });
                eprintln!("unknown command");
                sys_exit();
            } else {
                sys_waitpid(fork_result);
            }
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    sys_write(0, "\n\nDoglinkOS Shell v1.3 Snapshot 1228\n");
    shell_main_loop();
    if sys_fork() == 0 {
        // child
        TEST.t.set(5);
    } else {
        // parent
        TEST.t.set(4);
    }
    println!("Now TEST is {}!", TEST.t.get());
    sys_exec("/bin/exiter");
    sys_exit();
}
