# upppd PPP Test Flow

This document describes a minimal PPP-over-serial bring-up test between host `pppd` and guest `upppd`.

## Prerequisites

- A host system with `pppd` installed
- Permission to run `sudo pppd`
- This repository checked out and buildable on the host

## 1. Start QEMU With A PTY Serial Port

From the repository root on the host, run:

```sh
cargo run --release -- -b -S pty
```

QEMU prints a line like:

```text
char device redirected to /dev/pts/5 (label serial0)
```

Keep that PTY path for the next step.

## 2. Start `pppd` On The Host

In another host terminal, start `pppd` on the PTY reported by QEMU:

```sh
sudo pppd /dev/pts/5 115200 \
  10.1.1.1:10.1.1.2 \
  nodetach debug local persist silent noproxyarp noauth \
  noccp noipv6 novj novjccomp
```

Use the PTY path printed by your current QEMU run in place of `/dev/pts/5`.

## 3. Start `upppd` In The Guest

At the DoglinkOS-2nd shell inside the VM, run:

```sh
upppd
```

`upppd` uses `/dev/serial` as its PPP backend and remains running as the PPP service.

## 4. Verify Link Bring-Up

Successful host-side output includes:

```text
Using interface ppp0
local  IP address 10.1.1.1
remote IP address 10.1.1.2
```

You can also verify the host interface directly:

```sh
ip addr show ppp0
```

Expected addresses:

- host `ppp0`: `10.1.1.1`
- guest peer: `10.1.1.2`

## Notes

- The recommended host command disables `CCP`, `IPv6CP`, and Van Jacobson compression because this test only needs IPv4 IPCP.
- `upppd` previously dropped PPP frames when multiple frames arrived in one serial read. This flow verifies the fixed back-to-back frame handling path.
