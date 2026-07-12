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

fn print_help() {
    println!("Builtin commands:");
    println!("  help               Show this help text");
    println!("  panic-test         Trigger a panic for testing");
    println!("  exit               Exit the shell");
    println!("  sysinfo            Show system information");
    println!("  echo <text>        Print text");
    println!("  clear              Clear the screen");
    println!("  disk-read          Read from /dev/disk0");
    println!("  nvme-read          Read from /dev/nvme0-0");
    println!("  disk-size          Show /dev/disk0 size");
    println!("  nvme-size          Show /dev/nvme0-0 size");
    println!("  initrd-read        Read from /dev/initrd");
    println!("  file-read <path>   Print file contents");
    println!("  file-write <path>  Write lines to a file until EOF");
    println!("  mount <args>       Mount a filesystem");
    println!("  file-rm            Remove /test.txt");
    println!("  beep <freq>        Play a beep");
    println!("  poweroff           Power off the machine");
    println!("  reboot             Reboot the machine");
    println!();
    println!("External commands:");
    println!("  /bin/<name>        Execute a command from /bin");
    println!("  exiter             Do nothing");
    println!("  hello-std          A Rust std program that does not work properly");
    println!("  dins-empty         Do nothing");
    println!("  dins-hello         Print \"Hello, World!\"");
    println!("  pl_editor          An editor that can easily port to any OSes");
    println!(
        "  lua                A powerful, efficient, lightweight, embeddable scripting language"
    );
    println!("  huge-alloc-test    Memory allocation tester (requires at least 5 GiB of memory)");
    println!(
        "  imgview            Draw an image on the framebuffer (can mess up the terminal and become hard to clear)"
    );
}

fn shell_main_loop() {
    let mut buf = [0u8; 128];
    loop {
        print!("[User@DoglinkOS-2nd /]$ ");
        let len = read_line(&mut buf);
        let cmd = str::from_utf8(&buf[..len]).unwrap();
        if cmd.is_empty() {
            continue;
        } else if cmd == "help" {
            print_help();
        } else if cmd == "panic-test" {
            panic!("panic test");
        } else if cmd == "exit" {
            break;
        } else if cmd == "sysinfo" {
            println!("DoglinkOS-2nd version 1.4");
            println!("DoglinkOS Shell version 1.4");
            println!("In user mode");
            println!(
                "Console: {} rows, {} cols",
                sys_info(1).unwrap(),
                sys_info(0).unwrap()
            );
            println!(
                "Framebuffer: {} x {}, pitch {}",
                sys_info(6).unwrap(),
                sys_info(7).unwrap(),
                sys_info(9).unwrap()
            );
            println!("Current shell PID: {}", sys_info(2).unwrap());
            println!("Current kernel ticks: {}", sys_info(3).unwrap());
        } else if let Some(content) = cmd.strip_prefix("echo ") {
            println!("{content}");
        } else if cmd == "clear" {
            print!("\x1b[H\x1b[2J\x1b[3J");
        } else if cmd == "disk-read" {
            if let Some(fd) = sys_open("/dev/disk0", false) {
                let mut content = [0; 512];
                sys_read2(fd, &mut content);
                println!("{content:?}");
                sys_seek(fd, 0, SEEK_SET);
                sys_read2(fd, &mut content[..100]);
                sys_read2(fd, &mut content[100..]);
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
                sys_seek(fd, 0, SEEK_SET);
                sys_read2(fd, &mut content[..100]);
                sys_read2(fd, &mut content[100..]);
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
        } else if let Some(file_name) = cmd.strip_prefix("file-read ") {
            if let Some(fd) = sys_open(file_name, false) {
                let mut remaining_size = sys_seek(fd, 0, SEEK_END);
                sys_seek(fd, 0, SEEK_SET);
                let mut buf = [0; 512];
                while remaining_size > 0 {
                    let will_read = core::cmp::min(remaining_size, 512);
                    sys_read2(fd, &mut buf[..will_read]);
                    sys_write(1, str::from_utf8(&buf[..will_read]).unwrap());
                    remaining_size -= will_read;
                }
                sys_close(fd);
            } else {
                println!("file {file_name} not found");
            }
        } else if let Some(file_name) = cmd.strip_prefix("file-write ") {
            if let Some(fd) = sys_open(file_name, true) {
                let mut line_buf = [0u8; 128];
                while line_buf[0] != b'E' || line_buf[1] != b'O' || line_buf[2] != b'F' {
                    let len = read_line(&mut line_buf);
                    sys_write(fd, str::from_utf8(&line_buf[..len]).unwrap());
                    sys_write(fd, "\n");
                }
                sys_close(fd);
            } else {
                println!("error while opening {file_name}");
            }
        } else if let Some(params) = cmd.strip_prefix("mount ") {
            let mut it = params.split(' ');
            let typs = it.next().unwrap();
            let typ = if typs == "ahci" {
                0
            } else if typs == "nvme" {
                1
            } else {
                continue;
            };
            let disk: usize = it.next().unwrap().parse().unwrap();
            let part: usize = it.next().unwrap().parse().unwrap();
            let mountpoint = it.next().unwrap();
            if !mountpoint.ends_with('/') {
                eprintln!("a mount point must end with /");
            } else {
                sys_mount(typ, disk, part, mountpoint);
            }
        } else if cmd.starts_with("file-rm") {
            sys_remove("/test.txt");
        } else if let Some(freq) = cmd.strip_prefix("beep ") {
            if let Some(fd) = sys_open("/dev/pcspk", false) {
                sys_write(fd, freq);
                let start = sys_getticks();
                while sys_getticks() < start + 50 {}
                sys_write(fd, "stop");
                sys_close(fd);
            } else {
                println!("error while opening /dev/pcspk");
            }
        } else if cmd == "poweroff" || cmd == "reboot" {
            if let Some(fd) = sys_open("/dev/power", false) {
                sys_write(fd, cmd);
                sys_close(fd);
            } else {
                println!("error while opening /dev/power");
            }
        } else {
            let mut buf2 = [0u8; 128];
            let len2 = if buf[0] != b'/' {
                buf2[0..5].copy_from_slice(b"/bin/");
                buf2[5..(5 + len)].copy_from_slice(&buf[..len]);
                len + 5
            } else {
                buf2[..len].copy_from_slice(&buf[..len]);
                len
            };
            let fork_result = sys_fork();
            if fork_result == 0 {
                sys_exec(unsafe { core::str::from_utf8_unchecked(&buf2[..len2]) });
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
    sys_write(0, "\n\nDoglinkOS Shell v1.4\n");
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
