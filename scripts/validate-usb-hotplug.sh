#!/usr/bin/env bash
# Run the xHCI/BOT lifecycle baseline without a QEMU graphical window.
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
workdir=$(mktemp -d "${TMPDIR:-/tmp}/doglinkos-usb.XXXXXX")
qmp="$workdir/qmp.sock"
serial_log="$workdir/serial.log"
usb_image="$workdir/usb-hotplug.img"
qemu_pid=''

cleanup() {
    if [ -n "$qemu_pid" ]; then
        # `cargo run` starts builder, which starts QEMU.  A separate session
        # lets this single signal cover that whole process tree even after
        # cargo or builder has exited.
        kill -- "-$qemu_pid" 2>/dev/null || true
        wait "$qemu_pid" 2>/dev/null || true
    fi
    printf 'USB validation serial log: %s\n' "$serial_log"
}
trap cleanup EXIT

cd "$root"
cargo run -p builder -- --usb-storage-image "$usb_image"
setsid cargo run -p builder -- --boot --xhci-usb2-only --headless --serial-console --serial stdio --qmp "$qmp" >"$serial_log" 2>&1 &
qemu_pid=$!

for _ in $(seq 1 100); do
    [ -S "$qmp" ] && break
    sleep 0.1
done
[ -S "$qmp" ] || { cat "$serial_log"; exit 1; }

for _ in $(seq 1 600); do
    grep -Fq '[INFO] xhci: discovered ' "$serial_log" && break
    sleep 0.1
done
grep -Fq '[INFO] xhci: discovered ' "$serial_log" || { cat "$serial_log"; exit 1; }
if grep -Fq 'MSC BOT slot' "$serial_log"; then
    printf 'Unexpected MSC enumeration before QMP insertion\n' >&2
    cat "$serial_log" >&2
    exit 1
fi

# QMP owns all runtime changes.  Each device ID is stable so device_del is
# unambiguous and logs can be correlated with the guest port generation.
QMP_SOCKET="$qmp" SERIAL_LOG="$serial_log" USB_IMAGE="$usb_image" python3 - <<'PY'
import json
import os
import socket
import time

s = socket.socket(socket.AF_UNIX)
s.connect(os.environ["QMP_SOCKET"])
f = s.makefile("rwb", buffering=0)
json.loads(f.readline())

def command(execute, arguments=None):
    request = {"execute": execute}
    if arguments:
        request["arguments"] = arguments
    f.write(json.dumps(request).encode() + b"\n")
    while True:
        reply = json.loads(f.readline())
        if "event" not in reply:
            break
    if "error" in reply:
        raise RuntimeError(f"{execute}: {reply['error']}")

def wait_for_device_deleted(device):
    while True:
        reply = json.loads(f.readline())
        if (
            reply.get("event") == "DEVICE_DELETED"
            and reply.get("data", {}).get("device") == device
        ):
            return

def wait_for_serial(needle, timeout=45):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            if needle in open(os.environ["SERIAL_LOG"], encoding="utf-8", errors="replace").read():
                return
        except FileNotFoundError:
            pass
        time.sleep(0.1)
    raise RuntimeError(f"timed out waiting for guest log: {needle}")

command("qmp_capabilities")
image = os.environ["USB_IMAGE"]
for number in (1, 2):
    drive = f"usb-hotplug-drive-{number}"
    device = f"usb-hotplug-device-{number}"
    command("blockdev-add", {"node-name": drive, "driver": "raw",
            "read-only": True,
            "file": {"driver": "file", "filename": image}})
    command("device_add", {"driver": "usb-storage", "id": device,
            "drive": drive, "bus": "xhci.0"})
    if number == 1:
        # Do not remove a merely QMP-visible device: wait until the guest has
        # assigned a slot so `device_del` proves the xHCI removal path.
        wait_for_serial("MSC BOT slot")
        wait_for_serial("capacity ")
        wait_for_serial("add slot")
        # Remove and re-add the first disk to exercise slot/DMA reclamation.
        command("device_del", {"id": device})
        wait_for_device_deleted(device)
        command("device_add", {"driver": "usb-storage", "id": device,
                "drive": drive, "bus": "xhci.0"})
        time.sleep(2)
PY

# The driver's low-frequency lost-event fallback scans root ports every 256
# polls, so leave a full bounded scan interval after the QMP sequence. The
# expectations are log prefixes, not timing-sensitive complete log lines.
sleep 35
for expected in \
    '[INFO] xhci: port ' \
    'MSC BOT slot' \
    'capacity ' \
    'remove slot'; do
    if ! grep -Fq "$expected" "$serial_log"; then
        printf 'Missing serial assertion: %s\n' "$expected" >&2
        cat "$serial_log" >&2
        exit 1
    fi
done
if [ "$(grep -Fc 'MSC BOT slot' "$serial_log")" -lt 2 ]; then
    printf 'Missing two-device MSC enumeration evidence\n' >&2
    cat "$serial_log" >&2
    exit 1
fi

# QEMU's usb-storage model has no command-line BOT-reset fault injector.  CI
# supplies a fault-injection build when validating that path; its output must
# include this bounded-recovery log line in the saved serial artifact.
if [ "${USB_BOT_RESET_FAILURE_LOG:-}" ]; then
    grep -Fq "$USB_BOT_RESET_FAILURE_LOG" "$serial_log"
fi

printf 'USB hotplug validation passed\n'
