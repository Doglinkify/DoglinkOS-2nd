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
            println!("DoglinkOS-2nd version 1.3 Snapshot 0629");
            println!("DoglinkOS Shell version 1.3 Snapshot 0629");
            println!("In user mode");
        } else if cmd.starts_with("echo") {
            println!("{}", &cmd[5..]);
        } else if cmd == "hello-std" {
            sys_exec("/hello_std");
        } else if cmd == "dins empty" {
            sys_exec("/dins_empty");
        } else if cmd == "dins hello" {
            sys_exec("/dins_hello");
        } else {
            eprintln!("unknown command");
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    sys_write(0, "\n\nDoglinkOS Shell v1.3 Snapshot 0629\n");
    shell_main_loop();
    if sys_fork() == 0 {
        // child
        TEST.t.set(5);
    } else {
        // parent
        TEST.t.set(4);
    }
    println!("Now TEST is {}!", TEST.t.get());
    sys_exec("/exiter");
}
