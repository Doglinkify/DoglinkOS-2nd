use argh::FromArgs;
use builder::{FatBuilder, ImageBuilder, build_usb_storage_image};
use ovmf_prebuilt::{Arch, FileType, Prebuilt, Source};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(FromArgs)]
#[argh(description = "DoglinkOS-2nd bootloader and kernel builder")]
struct Args {
    #[argh(switch, short = 'b')]
    #[argh(description = "boot the constructed image")]
    boot: bool,

    #[argh(switch, short = 'k')]
    #[argh(description = "use KVM acceleration")]
    kvm: bool,

    #[argh(switch, short = 'w')]
    #[argh(description = "use Hyper-V acceleration")]
    whpx: bool,

    #[argh(option, short = 'c')]
    #[argh(default = "1")]
    #[argh(description = "number of CPU cores")]
    cores: usize,

    #[argh(option, short = 'm')]
    #[argh(description = "guest memory size")]
    memory: Option<String>,

    #[argh(switch, short = 'v')]
    #[argh(description = "use vnc")]
    vnc: bool,

    #[argh(switch, short = 'g')]
    #[argh(description = "debug mode")]
    debug: bool,

    #[argh(switch, short = 'n')]
    #[argh(description = "use nvme disk instead of ahci disk")]
    nvme: bool,

    #[argh(switch, short = 's')]
    #[argh(description = "enable sound card")]
    sound: bool,

    #[argh(option)]
    #[argh(default = "0")]
    #[argh(description = "PS/2 special cases")]
    ps2_special: usize,

    #[argh(option, short = 'S')]
    #[argh(description = "serial option passed to qemu")]
    serial: Option<String>,

    #[argh(switch)]
    #[argh(description = "attach the generated USB BOT test disk at startup")]
    usb_storage: bool,

    #[argh(switch)]
    #[argh(description = "add an empty qemu-xhci controller for QMP hotplug tests")]
    xhci: bool,

    #[argh(switch)]
    #[argh(description = "disable qemu-xhci USB 3 root ports (p3=0)")]
    xhci_usb2_only: bool,

    #[argh(option)]
    #[argh(description = "path for the generated USB BOT test disk")]
    usb_storage_image: Option<PathBuf>,

    #[argh(option)]
    #[argh(description = "QMP UNIX socket path passed to QEMU")]
    qmp: Option<String>,

    #[argh(switch)]
    #[argh(description = "disable the graphical display (use serial/QMP instead)")]
    headless: bool,

    #[argh(switch)]
    #[argh(description = "build with a serial+TTY kernel console for headless validation")]
    serial_console: bool,

    #[argh(switch)]
    #[argh(description = "emulate a Realtek RTL8139 network card instead of a default Intel one")]
    rtl_nic: bool,
}

fn main() {
    let args: Args = argh::from_env();
    let img_path = build_img(args.serial_console);
    let usb_storage_path = args.usb_storage_image.clone().unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("usb-hotplug-test.img")
    });
    if args.usb_storage || args.usb_storage_image.is_some() {
        build_usb_storage_image(&usb_storage_path).expect("failed to build USB storage test image");
        println!("Created USB BOT test image at {:#?}", usb_storage_path);
    }

    if args.boot {
        let mut cmd = Command::new("qemu-system-x86_64");

        let ovmf_path = Prebuilt::fetch(Source::LATEST, "target/ovmf")
            .expect("failed to update prebuilt")
            .get_file(Arch::X64, FileType::Code);
        let ovmf_config = format!("if=pflash,format=raw,file={}", ovmf_path.display());

        cmd.arg("-machine").arg("q35");
        cmd.arg("-drive").arg(ovmf_config);
        cmd.arg("-m").arg(args.memory.as_deref().unwrap_or("256m"));
        cmd.arg("-smp").arg(format!("cores={}", args.cores));
        cmd.arg("-cpu").arg("qemu64,+x2apic");

        if args.sound
            && let Some(backend) = match std::env::consts::OS {
                "linux" => Some("pa"),
                "macos" => Some("coreaudio"),
                "windows" => Some("dsound"),
                _ => None,
            }
        {
            cmd.arg("-audiodev").arg(format!("{backend},id=sound"));
            cmd.arg("-machine").arg("pcspk-audiodev=sound");
            cmd.arg("-device").arg("intel-hda");
            cmd.arg("-device").arg("hda-output,audiodev=sound");
        }

        if args.nvme {
            cmd.arg("-device").arg("nvme,drive=disk1,serial=deadbeef");
        } else {
            cmd.arg("-device").arg("ahci,id=ahci");
            cmd.arg("-device").arg("ide-hd,drive=disk1,bus=ahci.0");
        }
        let drive_config = format!("if=none,format=raw,id=disk1,file={}", img_path.display());
        cmd.arg("-drive").arg(drive_config);

        if args.kvm {
            cmd.arg("--enable-kvm");
        }
        if args.whpx {
            cmd.arg("-accel").arg("whpx");
        }
        if args.vnc {
            cmd.arg("-vnc").arg(":1");
        }
        if args.headless {
            cmd.arg("-display").arg("none");
        }
        if args.debug {
            cmd.arg("-s").arg("-S");
        }
        if let Some(qmp) = args.qmp {
            cmd.arg("-qmp")
                .arg(format!("unix:{qmp},server=on,wait=off"));
        }
        let needs_xhci = args.xhci
            || args.xhci_usb2_only
            || args.usb_storage
            || matches!(args.ps2_special, 2 | 3);
        if needs_xhci {
            // BOT storage is USB 2.0 in this validation suite.  Disable the
            // SuperSpeed root ports whenever it is attached at startup so a
            // controller/model port mapping cannot hide the USB 2 path.
            let xhci = if args.xhci_usb2_only || args.usb_storage {
                "qemu-xhci,id=xhci,p3=0"
            } else {
                "qemu-xhci,id=xhci"
            };
            cmd.arg("-device").arg(xhci);
        }
        if args.usb_storage {
            let drive = format!(
                "if=none,format=raw,readonly=on,id=usb-storage-drive,file={}",
                usb_storage_path.display()
            );
            cmd.arg("-drive").arg(drive);
            cmd.arg("-device")
                .arg("usb-storage,id=usb-storage-start,drive=usb-storage-drive,bus=xhci.0");
        }
        match args.ps2_special {
            1 => _ = cmd.arg("-machine").arg("i8042=off"),
            2 => _ = cmd.arg("-device").arg("usb-kbd,bus=xhci.0"),
            3 => _ = cmd.arg("-device").arg("usb-mouse,bus=xhci.0"),
            _ => {}
        }
        if let Some(opt) = args.serial {
            cmd.arg("-serial").arg(opt);
        }
        if args.rtl_nic {
            cmd.arg("-device").arg("rtl8139,netdev=net0");
            cmd.arg("-netdev").arg("user,id=net0");
        }

        let mut child = cmd.spawn().unwrap();
        child.wait().unwrap();
    }
}

fn build_img(serial_console: bool) -> PathBuf {
    let doglinked_path = Path::new(env!("CARGO_BIN_FILE_DOGLINKED"));
    let t_path = Path::new(env!("CARGO_BIN_FILE_INFINITE_LOOP"));
    let imgview_path = Path::new(env!("CARGO_BIN_FILE_IMGVIEW"));
    let ipc_demo_path = Path::new(env!("CARGO_BIN_FILE_IPC_DEMO"));
    let upppd_path = Path::new(env!("CARGO_BIN_FILE_UPPPD"));

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let assets_dir = manifest_dir.join("assets");

    let initrd_files = BTreeMap::from([
        ("/sbin/doglinked", doglinked_path.to_path_buf()),
        ("/bin/exiter", t_path.to_path_buf()),
        ("/bin/hello-std", assets_dir.join("hello_std.elf")),
        ("/bin/dins-empty", assets_dir.join("empty.elf")),
        ("/bin/dins-hello", assets_dir.join("hello.elf")),
        ("/bin/pl_editor", assets_dir.join("pl_editor.elf")),
        ("/bin/lua", assets_dir.join("lua.elf")),
        ("/bin/huge-alloc-test", assets_dir.join("huge_alloc.elf")),
        ("/bin/imgview", imgview_path.to_path_buf()),
        ("/bin/ipc-demo", ipc_demo_path.to_path_buf()),
        ("/bin/upppd", upppd_path.to_path_buf()),
        ("/bin/videoplay", assets_dir.join("videoplay.elf")),
        ("/res/test.jpg", assets_dir.join("test.jpg")),
        ("/res/test2.jpg", assets_dir.join("test2.jpg")),
        ("/res/test2.png", assets_dir.join("test2.png")),
        ("/res/test2_16.png", assets_dir.join("test2_16.png")),
        ("/res/test2.ppm", assets_dir.join("test2.ppm")),
        ("/res/test2.qoi", assets_dir.join("test2.qoi")),
        ("/res/demo.dlv", assets_dir.join("demo.dlv")),
        ("/res/demo-dlv2.dlv", assets_dir.join("demo-dlv2.dlv")),
        ("/res/demo.avi", assets_dir.join("demo-mjpeg.avi")),
        ("/res/demo2.avi", assets_dir.join("demo2.avi")),
    ]);
    let initrd_path = manifest_dir.parent().unwrap().join("initrd.img");
    FatBuilder::create(initrd_files, &initrd_path).expect("failed to build initrd.img");
    println!("Created initrd.img at {:#?}", initrd_path);

    let kernel_path = Path::new(env!("CARGO_BIN_FILE_DOGLINKOS_2ND"));
    println!("Building UEFI disk image for kernel at {:#?}", kernel_path);

    let files = BTreeMap::from([
        ("kernel", kernel_path.to_path_buf()),
        ("efi/boot/bootx64.efi", assets_dir.join("BOOTX64.EFI")),
        (
            "limine.conf",
            assets_dir.join(if serial_console {
                "limine-serial.conf"
            } else {
                "limine.conf"
            }),
        ),
        ("initrd.img", initrd_path.to_path_buf()),
    ]);

    let img_path = manifest_dir.parent().unwrap().join("DoglinkOS-2nd.img");
    ImageBuilder::build(files, &img_path).expect("Failed to build UEFI disk image");
    println!("Created bootable UEFI disk image at {:#?}", img_path);

    img_path
}
