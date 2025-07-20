use argh::FromArgs;
use builder::{FatBuilder, ImageBuilder};
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

    #[argh(switch, short = 'v')]
    #[argh(description = "use vnc")]
    vnc: bool,
}

fn main() {
    let img_path = build_img();
    let args: Args = argh::from_env();

    if args.boot {
        let mut cmd = Command::new("qemu-system-x86_64");

        let ovmf_path = Prebuilt::fetch(Source::LATEST, "target/ovmf")
            .expect("failed to update prebuilt")
            .get_file(Arch::X64, FileType::Code);
        let ovmf_config = format!("if=pflash,format=raw,file={}", ovmf_path.display());

        cmd.arg("-machine").arg("q35");
        cmd.arg("-drive").arg(ovmf_config);
        cmd.arg("-m").arg("256m");
        cmd.arg("-smp").arg(format!("cores={}", args.cores));
        cmd.arg("-cpu").arg("qemu64,+x2apic");

        // if let Some(backend) = match std::env::consts::OS {
        //     "linux" => Some("pa"),
        //     "macos" => Some("coreaudio"),
        //     "windows" => Some("dsound"),
        //     _ => None,
        // } {
        //     cmd.arg("-audiodev").arg(format!("{},id=sound", backend));
        //     cmd.arg("-machine").arg("pcspk-audiodev=sound");
        //     cmd.arg("-device").arg("intel-hda");
        //     cmd.arg("-device").arg("hda-output,audiodev=sound");
        // }

        let drive_config = format!("if=none,format=raw,id=disk1,file={}", img_path.display());
        cmd.arg("-device").arg("ahci,id=ahci");
        cmd.arg("-device").arg("ide-hd,drive=disk1,bus=ahci.0");
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

        let mut child = cmd.spawn().unwrap();
        child.wait().unwrap();
    }
}

fn build_img() -> PathBuf {
    let doglinked_path = Path::new(env!("CARGO_BIN_FILE_DOGLINKED"));
    let t_path = Path::new(env!("CARGO_BIN_FILE_INFINITE_LOOP"));

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let assets_dir = manifest_dir.join("assets");

    let initrd_files = BTreeMap::from([
        ("/sbin/doglinked", doglinked_path.to_path_buf()),
        ("/bin/exiter", t_path.to_path_buf()),
        ("/bin/hello-std", assets_dir.join("hello_std.elf")),
        ("/bin/dins-empty", assets_dir.join("empty.elf")),
        ("/bin/dins-hello", assets_dir.join("hello.elf")),
        ("/bin/pl_editor", assets_dir.join("pl_editor.elf")),
    ]);
    let initrd_path = manifest_dir.parent().unwrap().join("initrd.img");
    FatBuilder::create(initrd_files, &initrd_path).expect("failed to build initrd.img");
    println!("Created initrd.img at {:#?}", &initrd_path);

    let kernel_path = Path::new(env!("CARGO_BIN_FILE_DOGLINKOS_2ND"));
    println!("Building UEFI disk image for kernel at {:#?}", &kernel_path);

    let files = BTreeMap::from([
        ("kernel", kernel_path.to_path_buf()),
        ("efi/boot/bootx64.efi", assets_dir.join("BOOTX64.EFI")),
        ("limine.conf", assets_dir.join("limine.conf")),
        ("initrd.img", initrd_path.to_path_buf()),
    ]);

    let img_path = manifest_dir.parent().unwrap().join("DoglinkOS-2nd.img");
    ImageBuilder::build(files, &img_path).expect("Failed to build UEFI disk image");
    println!("Created bootable UEFI disk image at {:#?}", &img_path);

    img_path
}
