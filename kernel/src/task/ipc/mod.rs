use crate::task::process::{ProcessContext, TASKS};
use crate::task::sched::CURRENT_TASK_ID;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::min;
use core::sync::atomic::Ordering;
use spin::Mutex;

pub const IPC_MAX_HANDLES: usize = 64;
pub const IPC_MAX_MSG_SIZE: usize = 4096;
pub const IPC_QUEUE_DEPTH: usize = 16;

pub const IPC_FLAG_NONBLOCK: usize = 1;

pub const IPC_CMD_CREATE: usize = 0;
pub const IPC_CMD_SEND: usize = 1;
pub const IPC_CMD_RECV: usize = 2;
pub const IPC_CMD_CLOSE: usize = 3;
pub const IPC_CMD_DUP: usize = 4;

const IPC_OK: isize = 0;
const IPC_EINVAL: isize = -22;
const IPC_EBADF: isize = -9;
const IPC_EAGAIN: isize = -11;
const IPC_EMFILE: isize = -24;
const IPC_EPIPE: isize = -32;
const IPC_EMSGSIZE: isize = -90;

pub type IpcHandle = Arc<Mutex<IpcHandleState>>;

pub struct IpcHandleState {
    channel: Arc<Mutex<IpcChannel>>,
    side: usize,
}

struct IpcChannel {
    refs: [usize; 2],
    endpoints: [IpcEndpoint; 2],
}

struct IpcEndpoint {
    queue: VecDeque<IpcMessage>,
    closed: bool,
}

struct IpcMessage {
    data: Vec<u8>,
}

impl IpcEndpoint {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            closed: false,
        }
    }
}

impl IpcChannel {
    fn new() -> Self {
        Self {
            refs: [0, 0],
            endpoints: [IpcEndpoint::new(), IpcEndpoint::new()],
        }
    }
}

pub fn clone_handle_table(
    handles: &[Option<IpcHandle>; IPC_MAX_HANDLES],
) -> [Option<IpcHandle>; IPC_MAX_HANDLES] {
    core::array::from_fn(|idx| handles[idx].as_ref().map(dup_handle_ref))
}

pub fn release_handle_table(handles: &mut [Option<IpcHandle>; IPC_MAX_HANDLES]) {
    for slot in handles.iter_mut() {
        if let Some(handle) = slot.take() {
            close_handle_ref(handle);
        }
    }
}

pub fn syscall(args: &mut ProcessContext) {
    let ret = match args.rdi as usize {
        IPC_CMD_CREATE => sys_create(args),
        IPC_CMD_SEND => sys_send(args),
        IPC_CMD_RECV => sys_recv(args),
        IPC_CMD_CLOSE => sys_close(args),
        IPC_CMD_DUP => sys_dup(args),
        _ => IPC_EINVAL,
    };
    args.rax = ret as u64;
}

fn sys_create(args: &mut ProcessContext) -> isize {
    let _flags = args.rsi as usize;
    let channel = Arc::new(Mutex::new(IpcChannel::new()));
    let handle0 = Arc::new(Mutex::new(IpcHandleState {
        channel: channel.clone(),
        side: 0,
    }));
    let handle1 = Arc::new(Mutex::new(IpcHandleState { channel, side: 1 }));
    {
        let inner = handle0.lock();
        inner.channel.lock().refs[0] += 1;
    }
    {
        let inner = handle1.lock();
        inner.channel.lock().refs[1] += 1;
    }

    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    let slots = {
        let mut tasks = TASKS.lock();
        let task = tasks[current].as_mut().unwrap();
        let mut free = task
            .ipc_handles
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| entry.is_none().then_some(idx));
        match (free.next(), free.next()) {
            (Some(slot0), Some(slot1)) => {
                task.ipc_handles[slot0] = Some(handle0.clone());
                task.ipc_handles[slot1] = Some(handle1.clone());
                Some((slot0, slot1))
            }
            _ => None,
        }
    };
    let Some((slot0, slot1)) = slots else {
        close_handle_ref(handle0);
        close_handle_ref(handle1);
        return IPC_EMFILE;
    };
    args.rdx = slot1 as u64;
    slot0 as isize
}

fn sys_send(args: &mut ProcessContext) -> isize {
    let handle_id = args.rsi as usize;
    let ptr = args.rdx as *const u8;
    let len = args.rcx as usize;
    let _flags = args.r8 as usize;
    if len > IPC_MAX_MSG_SIZE {
        return IPC_EMSGSIZE;
    }
    let buf = unsafe { core::slice::from_raw_parts(ptr, len) };
    let Some((channel, side)) = current_handle(handle_id) else {
        return IPC_EBADF;
    };
    let dest = side ^ 1;
    let mut locked = channel.lock();
    if locked.endpoints[side].closed || locked.endpoints[dest].closed {
        return IPC_EPIPE;
    }
    let endpoint = &mut locked.endpoints[dest];
    if endpoint.queue.len() >= IPC_QUEUE_DEPTH {
        return IPC_EAGAIN;
    }
    endpoint.queue.push_back(IpcMessage { data: buf.to_vec() });
    len as isize
}

fn sys_recv(args: &mut ProcessContext) -> isize {
    let handle_id = args.rsi as usize;
    let ptr = args.rdx as *mut u8;
    let len = args.rcx as usize;
    let _flags = args.r8 as usize;
    let Some((channel, side)) = current_handle(handle_id) else {
        return IPC_EBADF;
    };
    let mut locked = channel.lock();
    let peer_closed = locked.endpoints[side ^ 1].closed;
    let endpoint = &mut locked.endpoints[side];
    if let Some(message) = endpoint.queue.pop_front() {
        let copy_len = min(len, message.data.len());
        unsafe {
            core::ptr::copy_nonoverlapping(message.data.as_ptr(), ptr, copy_len);
        }
        copy_len as isize
    } else if peer_closed || endpoint.closed {
        IPC_OK
    } else {
        IPC_EAGAIN
    }
}

fn sys_close(args: &mut ProcessContext) -> isize {
    let handle_id = args.rsi as usize;
    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    let handle = {
        let mut tasks = TASKS.lock();
        let task = tasks[current].as_mut().unwrap();
        if handle_id >= task.ipc_handles.len() {
            return IPC_EBADF;
        }
        task.ipc_handles[handle_id].take()
    };
    match handle {
        Some(handle) => {
            close_handle_ref(handle);
            IPC_OK
        }
        None => IPC_EBADF,
    }
}

fn sys_dup(args: &mut ProcessContext) -> isize {
    let handle_id = args.rsi as usize;
    let Some(source) = current_handle_ref(handle_id) else {
        return IPC_EBADF;
    };
    let duped = dup_handle_ref(&source);
    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    let slot = {
        let mut tasks = TASKS.lock();
        let task = tasks[current].as_mut().unwrap();
        task.ipc_handles
            .iter()
            .position(Option::is_none)
            .map(|slot| {
                task.ipc_handles[slot] = Some(duped.clone());
                slot
            })
    };
    let Some(slot) = slot else {
        close_handle_ref(duped);
        return IPC_EMFILE;
    };
    slot as isize
}

fn current_handle(handle_id: usize) -> Option<(Arc<Mutex<IpcChannel>>, usize)> {
    let handle = current_handle_ref(handle_id)?;
    let locked = handle.lock();
    Some((locked.channel.clone(), locked.side))
}

fn current_handle_ref(handle_id: usize) -> Option<IpcHandle> {
    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    let tasks = TASKS.lock();
    let task = tasks[current].as_ref()?;
    task.ipc_handles.get(handle_id)?.as_ref().cloned()
}

fn dup_handle_ref(handle: &IpcHandle) -> IpcHandle {
    let (channel, side) = {
        let locked = handle.lock();
        (locked.channel.clone(), locked.side)
    };
    {
        let mut locked = channel.lock();
        locked.refs[side] += 1;
    }
    handle.clone()
}

fn close_handle_ref(handle: IpcHandle) {
    let (channel, side) = {
        let locked = handle.lock();
        (locked.channel.clone(), locked.side)
    };
    let mut locked = channel.lock();
    if locked.refs[side] != 0 {
        locked.refs[side] -= 1;
        if locked.refs[side] == 0 {
            close_side(&mut locked, side);
        }
    }
}

fn close_side(channel: &mut IpcChannel, side: usize) {
    let endpoint = &mut channel.endpoints[side];
    endpoint.closed = true;
    endpoint.queue.clear();
}
