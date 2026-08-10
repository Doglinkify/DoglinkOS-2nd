# xHCI support and validation

## Support scope

DoglinkOS-2nd supports a PCIe xHCI controller on x86_64 UEFI systems for a
USB 2.0 HID Boot keyboard or mouse connected directly to a root-hub port.
Keyboard reports are translated into Set-1 make and break scan codes and use
the same TTY and `INPUT_BUFFER` path as PS/2. Mouse reports use the common
mouse submission interface, including scroll events.

The driver uses 64-bit DMA addresses and enables a single-vector PCI MSI
notification when the controller exposes a usable MSI capability. Missing or
invalid MSI capability falls back to bounded polling; event parsing stays in
the kernel idle path in either mode. No xHCI controller, no attached USB
device, and controller initialization failures are non-fatal.

This release does not support USB storage, USB 3.x SuperSpeed data paths,
external hubs, isochronous transfers, a generic HID report-descriptor parser,
hot-unplug resource reclamation, IOMMUs, MSI-X, or controller load balancing.
It assumes the DMA physical addresses are directly usable by the controller
because no IOMMU is configured.

## QEMU startup

Use the workspace builder to create and boot an image with one USB input
device:

```bash
cargo run -p builder -- --boot --ps2-special 2 --serial stdio
cargo run -p builder -- --boot --ps2-special 3 --serial stdio
```

`--ps2-special 2` adds `qemu-xhci` and `usb-kbd`; `--ps2-special 3` adds
`qemu-xhci` and `usb-mouse`. Focus the QEMU window before entering keys or
using the mouse wheel. The expected log sequence includes controller reset,
the USB 2 root-port reset, device slot assignment, HID endpoint configuration,
and one MSI configuration message or an explicit polling-fallback message.

For a headless keyboard test, temporarily set the Limine command line in
`builder/assets/limine.conf` to `stdio=serial+tty`, rebuild the image, and run
QEMU with a serial log and monitor socket. Restore `stdio=tty` after the
test. Send `sendkey a` through the monitor and verify that `a` appears at the
TTY prompt in the serial log. This setting is necessary because `stdio=serial`
alone does not deliver TTY input.

## Validation matrix

| Target | Device and command | Required observation | Evidence status |
| --- | --- | --- | --- |
| QEMU q35 | `qemu-xhci` + `usb-kbd`; `cargo run -p builder -- --boot --ps2-special 2 --serial stdio` | HID Boot keyboard endpoint configures; letter, Shift, Enter, Backspace, and release events reach TTY without duplicates. | Verified on 2026-08-10 with QEMU 11.0.2: boot, endpoint configuration, and monitor-injected `a`, Shift-A, Enter, and Backspace reached TTY without a kernel fault. Retain the serial log with the release artifact. |
| QEMU q35 | `qemu-xhci` + `usb-mouse`; `cargo run -p builder -- --boot --ps2-special 3 --serial stdio` | HID Boot mouse endpoint configures; wheel behavior matches PS/2; no transfer timeout or ring overflow. | Boot and endpoint configuration verified on 2026-08-10 with QEMU 11.0.2. Wheel behavior requires a graphical/manual run before release. |
| QEMU q35 | No xHCI device | Kernel boots normally and reports zero discovered xHCI controllers. | Regress before release. |
| QEMU q35 or hardware | PS/2 keyboard and mouse only | Existing PS/2 typing and scrolling remain functional. | Regress before release. |
| Physical x86_64 UEFI host | Native PCIe xHCI controller and direct USB 2 HID Boot keyboard/mouse | Cold boot, enumeration, input, MSI/polling mode, and PS/2 coexistence complete without fault. | No reproducible physical-host result is recorded in this repository yet; this remains a release blocker. |

The QEMU rows are the maintained regression baseline. A physical-host result
must name the machine or controller, PCI BDF, firmware/boot mode, device,
kernel commit, test date, and saved serial log or CI artifact before the
physical row can be marked verified.

## Failure reports and regression

Attach the serial log to every xHCI failure report. Record the controller BDF,
BAR0 address and length when mapping fails, root port, slot, endpoint address,
USB speed, operation stage, completion code, and `USBSTS` where available.
Also state whether MSI was configured or polling fallback was selected. These
fields make reset, command, transfer, and descriptor failures actionable.

Before merging an xHCI change, run `cargo fmt --all -- --check` and
`cargo check -p DoglinkOS-2nd --target x86_64-unknown-none`, then cover the
keyboard, mouse, no-xHCI, and PS/2-only matrix rows. Repeat cold boot three
times and verify that logs contain no kernel fault, infinite timeout loop,
DMA-alignment warning, duplicate input, transfer timeout, ring overflow, or
storage/PS/2 regression. Run the physical-host cold-boot case for release
validation and retain its evidence with the change.
