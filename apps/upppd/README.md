# upppd IPC Client Protocol

`upppd` is a user-space PPPoS service.

- Service name: `upppd`
- Transport: DoglinkOS-2nd named IPC
- Serial backend: `/dev/serial`
- PPP implementation: `ppproto`

This document is for programs that want to talk to `upppd` over IPC.

## Overview

After a client connects to the named IPC port `upppd`, the service keeps that connection open and uses it bidirectionally:

- client -> `upppd`: request to send an IPv4 packet, or query link status
- `upppd` -> client: link status events, send acknowledgements, received IPv4 packets, and error notifications

Each IPC message is one complete request or event. There is no stream framing inside the IPC payload.

## Connect

Clients should:

1. Call `sys_ipc_connect("upppd")`
2. Keep the returned handle open
3. Start receiving messages from that handle

Immediately after a successful connection, `upppd` sends one status event so the client can learn the current PPP state without sending a query first.

## Request Messages

### `0x01`: Send IPv4 packet

Format:

```text
+--------+-------------------+
| byte 0 | bytes 1..N        |
+--------+-------------------+
| 0x01   | raw IPv4 packet   |
+--------+-------------------+
```

Notes:

- The payload must be a complete IPv4 packet, starting from the IPv4 header.
- `upppd` wraps it into a PPP IPv4 frame and sends it on the serial line.
- On success, the client receives an `EVT_ACK` event.
- If PPP encoding fails, the client receives an `EVT_ERROR` event.

### `0x02`: Query current status

Format:

```text
+--------+
| byte 0 |
+--------+
| 0x02   |
+--------+
```

Response:

- `upppd` replies with one `EVT_STATUS` event.

## Event Messages

### `0x80`: ACK

Format:

```text
+--------+--------+-------------------+
| byte 0 | byte 1 | bytes 2..31       |
+--------+--------+-------------------+
| 0x80   | code   | reserved, zero    |
+--------+--------+-------------------+
```

Current behavior:

- `code == 0` means success
- other values are currently unused

### `0x81`: Received IPv4 packet

Format:

```text
+--------+-------------------+
| byte 0 | bytes 1..N        |
+--------+-------------------+
| 0x81   | raw IPv4 packet   |
+--------+-------------------+
```

Notes:

- The payload is a complete IPv4 packet extracted from an incoming PPP frame.
- `upppd` broadcasts this event to every connected IPC client.
- Clients that do not want unsolicited packets must still keep draining the IPC queue, or their queue may fill up.

### `0x82`: Link status

Format:

```text
+--------+--------+--------+--------+--------+--------+--------+
| byte 0 | byte 1 | 2..5   | 6..9   | 10..13 | 14..17 | 18..31 |
+--------+--------+--------+--------+--------+--------+--------+
| 0x82   | phase  | local  | peer   | dns1   | dns2   | zero   |
+--------+--------+--------+--------+--------+--------+--------+
```

This is currently a fixed 32-byte message.

Field definitions:

- `byte 1`: PPP phase
- `bytes 2..5`: local IPv4 address
- `bytes 6..9`: peer IPv4 address
- `bytes 10..13`: DNS server 1
- `bytes 14..17`: DNS server 2
- `bytes 18..31`: reserved, currently zero

IP address encoding:

- IPv4 addresses are stored as four raw octets in network order
- `0.0.0.0` means "not available yet"

Phase values:

- `0`: `Dead`
- `1`: `Establish`
- `2`: `Auth`
- `3`: `Network`
- `4`: `Open`

When status is sent:

- immediately after a client connects
- whenever the PPP phase changes
- when the client explicitly sends request `0x02`

### `0xFF`: Error

Format:

```text
+--------+--------+-------------------+
| byte 0 | byte 1 | bytes 2..31       |
+--------+--------+-------------------+
| 0xFF   | code   | reserved, zero    |
+--------+--------+-------------------+
```

Current error codes:

- `1`: empty request
- `2`: failed to encode/send outbound IPv4 packet into PPP
- `3`: unknown request opcode

## Expected Client Behavior

Clients should treat the IPC channel as asynchronous:

- a status event may arrive at any time
- received IPv4 packets may arrive at any time
- a response to a request is not tagged with a request ID

Because there is no request ID in the current protocol, a client should avoid sending multiple requests and then trying to match responses out of order. The safest model is:

1. send one request
2. keep receiving until the expected reply arrives
3. then send the next request

## Minimal Exchange

Typical startup sequence:

1. `sys_ipc_connect("upppd")`
2. receive initial `0x82` status event
3. optionally send `0x02` if the client wants a fresh status snapshot
4. wait until phase becomes `4` (`Open`)
5. send `0x01 + ipv4_packet`
6. receive `0x80` ack
7. keep receiving `0x81` packets and later `0x82` status changes

## Limits And Current Semantics

- IPC max message size in the kernel is currently `4096` bytes
- `upppd` uses a `4096`-byte receive buffer for client IPC messages
- very large IPv4 packets will not fit if they exceed IPC limits
- `upppd` currently broadcasts inbound IPv4 packets to all connected clients
- `upppd` does not currently multiplex packets by protocol, socket, or session
- `upppd` currently starts PPP with empty PAP username/password

## Example Pseudocode

```rust
let handle = connect_named_ipc("upppd");

loop {
    let msg = recv_ipc(handle);
    match msg[0] {
        0x82 => {
            let phase = msg[1];
            if phase == 4 {
                break;
            }
        }
        _ => {}
    }
}

let mut req = Vec::new();
req.push(0x01);
req.extend_from_slice(&ipv4_packet);
send_ipc(handle, &req);

loop {
    let msg = recv_ipc(handle);
    match msg[0] {
        0x80 => break,
        0x81 => handle_inbound_ipv4(&msg[1..]),
        0x82 => handle_status(&msg),
        0xFF => handle_error(msg[1]),
        _ => {}
    }
}
```
