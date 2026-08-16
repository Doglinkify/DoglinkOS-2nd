# xHCI support and validation

## Support scope

DoglinkOS-2nd supports a PCIe xHCI controller on x86_64 UEFI systems for
USB 2.0 devices connected directly to a root-hub port:

- HID Boot keyboards and mice;
- Mass Storage / SCSI Transparent Command Set devices using Bulk-Only
  Transport (BOT), exposed as read-only `/dev/usbN` block devices;
- connect, disconnect, and reinsertion of those devices on a USB 2 root port.

Keyboard reports are translated into Set-1 make and break scan codes and use
the same TTY and `INPUT_BUFFER` path as PS/2. Mouse reports use the common
mouse submission interface, including scroll events.

On removal, the driver stops submissions, offlines the corresponding block
device, disables the xHCI slot, and reclaims its xHCI DMA resources only after
the controller has relinquished them. New opens of a removed `/dev/usbN` fail;
existing handles report that the USB storage device was removed. A later device
is a new generation and is never rebound to an old handle.

The driver uses 64-bit DMA addresses and enables a single-vector PCI MSI
notification when the controller exposes a usable MSI capability. Missing or
invalid MSI capability falls back to bounded polling; event parsing stays in
the kernel idle path in either mode. No xHCI controller, no attached USB
device, and controller initialization failures are non-fatal.

Storage support is deliberately narrow: USB 2 direct root-port devices, one
LUN, 512-byte logical blocks, `GET_MAX_LUN`, `INQUIRY`, `TEST UNIT READY`,
`REQUEST SENSE`, `READ CAPACITY(10)`, and segmented read-only `READ(10)`.
CBW/CSW validation and bounded BOT reset plus bulk-endpoint clear-halt recovery
are implemented. Writes, non-512-byte blocks, `READ CAPACITY(16)`, multiple
LUNs, UAS, USB 3.x SuperSpeed data paths, external hubs, isochronous transfers,
a generic HID report-descriptor parser, suspend/resume, IOMMUs, MSI-X, and
controller load balancing remain unsupported. DMA physical addresses must be
directly usable by the controller because no IOMMU is configured.

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

## Headless USB storage and hotplug validation

The builder can generate a deterministic GPT test image with one FAT
partition and attach it as a BOT device at boot. `--usb-storage` automatically
disables the QEMU USB 3 root ports (`p3=0`); `usb-storage-start` is the startup
device ID and `usb-storage-drive` is its QEMU drive ID:

```bash
cargo run -p builder -- --boot --usb-storage --headless --serial-console --serial stdio
```

For repeatable runtime validation, run the non-graphical QMP harness:

```bash
scripts/validate-usb-hotplug.sh
```

It creates the USB image in a temporary directory, starts QEMU with the QMP
UNIX socket and no USB device, then uses these QMP IDs in order:

| Scenario | QMP IDs / command | Required serial assertion |
| --- | --- | --- |
| No device | Initial boot with `qemu-xhci,id=xhci,p3=0` only (`builder --xhci-usb2-only`) | xHCI initializes without an MSC line. |
| Runtime insertion | `blockdev-add` `usb-hotplug-drive-1`; `device_add` `usb-hotplug-device-1` on `xhci.0` | `MSC BOT slot` and `capacity` appear. |
| Runtime removal/reinsertion | `device_del usb-hotplug-device-1`, wait for `DEVICE_DELETED`, then re-add it | `remove slot` appears and a later MSC enumeration succeeds. |
| Two devices | add `usb-hotplug-drive-2` / `usb-hotplug-device-2` while device 1 is present | two MSC enumeration/capacity records appear. |

The script retains the serial-log path on exit and fails if its lifecycle log
assertions are missing. Set `USB_BOT_RESET_FAILURE_LOG` in CI to the expected
bounded-recovery line when running a QEMU or kernel fault-injection build that
forces a BOT reset failure; upstream QEMU's `usb-storage` device does not
provide a BOT-reset failure injector. This keeps the failure-path assertion
explicit rather than treating a manually observed QMP error as BOT recovery.

For a headless keyboard test, pass `--serial-console` to select the dedicated
`stdio=serial+tty` Limine configuration, then run QEMU with a serial log and
monitor socket. Send `sendkey a` through the monitor and verify that `a`
appears at the TTY prompt in the serial log. This setting is necessary because
`stdio=serial` alone does not deliver TTY input.

## Verified environments and validation matrix

| Target | Device and command | Required observation | Evidence status |
| --- | --- | --- | --- |
| QEMU q35 | `qemu-xhci` + `usb-kbd`; `cargo run -p builder -- --boot --ps2-special 2 --serial stdio` | HID Boot keyboard endpoint configures; letter, Shift, Enter, Backspace, and release events reach TTY without duplicates. | Verified on 2026-08-10 with QEMU 11.0.2: boot, endpoint configuration, and monitor-injected `a`, Shift-A, Enter, and Backspace reached TTY without a kernel fault. Retain the serial log with the release artifact. |
| QEMU q35 | `qemu-xhci` + `usb-mouse`; `cargo run -p builder -- --boot --ps2-special 3 --serial stdio` | HID Boot mouse endpoint configures; wheel behavior matches PS/2; no transfer timeout or ring overflow. | Boot and endpoint configuration verified on 2026-08-10 with QEMU 11.0.2. Wheel behavior requires a graphical/manual run before release. |
| QEMU q35 | `qemu-xhci` USB 2 ports (`p3=0`) + QEMU `usb-storage` BOT device; `scripts/validate-usb-hotplug.sh` | Startup storage, runtime insert/remove/reinsert, and two-device enumeration produce the documented MSC, capacity, and slot-removal log records. | Repeatable headless QMP harness supplied. Preserve its serial-log artifact when recording a verified QEMU/version result. |
| QEMU q35 | No xHCI device | Kernel boots normally and reports zero discovered xHCI controllers. | Regress before release. |
| QEMU q35 or hardware | PS/2 keyboard and mouse only | Existing PS/2 typing and scrolling remain functional. | Regress before release. |
| Physical x86_64 UEFI host | Native PCIe xHCI controller; direct USB 2 HID Boot keyboard/mouse and BOT storage | Cold boot, input/storage I/O, hotplug, removal, reinsertion, and PS/2 coexistence complete without fault. | No reproducible physical-host result is recorded in this repository yet; this remains a release blocker. |

The maintained baseline is QEMU q35 with its emulated `qemu-xhci` controller;
the HID evidence above was collected with QEMU 11.0.2. The `usb-kbd`,
`usb-mouse`, and QEMU `usb-storage` BOT models are the documented regression
devices. A physical-host result must name the machine or controller, PCI BDF,
firmware/boot mode, device VID:PID, device protocol/speed, kernel commit, test
date, and saved serial log or CI artifact before the physical row can be marked
verified.

## Failure reports and regression

Attach the complete serial log to every xHCI failure report; for QMP runs also
attach the QMP command transcript and the `validate-usb-hotplug.sh` log path.
Record the controller BDF and xHCI version, BAR0 address and length when
mapping fails, root port, port generation, slot, device kind, endpoint address,
USB speed, operation stage, completion code, and `USBSTS` where available.
For storage failures also record the BOT command/tag, SCSI opcode, LUN, LBA,
transfer length, CSW status/residue, and whether reset/clear-halt recovery was
attempted. State whether MSI was configured or polling fallback was selected,
and include the device VID:PID when descriptor reads completed. These fields
make reset, enumeration, transfer, hot-unplug, and BOT failures actionable.

Before merging an xHCI change, run `cargo fmt --all -- --check` and
`cargo check -p DoglinkOS-2nd --target x86_64-unknown-none`, then cover the
keyboard, mouse, no-xHCI, PS/2-only, and applicable storage/hotplug matrix
rows. Repeat cold boot three times and verify that logs contain no kernel
fault, infinite timeout loop, DMA-alignment warning, duplicate input, transfer
timeout, ring overflow, slot leak, stale-device I/O, or storage/PS/2 regression.
Run the physical-host cold-boot and hotplug cases for release validation and
retain their evidence with the change.
