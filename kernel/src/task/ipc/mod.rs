use crate::task::process::{ProcessContext, TASKS};
use crate::task::sched::CURRENT_TASK_ID;
use alloc::borrow::ToOwned;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::min;
use core::sync::atomic::Ordering;
use spin::{Lazy, Mutex};

pub const IPC_MAX_HANDLES: usize = 64;
pub const IPC_MAX_MSG_SIZE: usize = 4096;
pub const IPC_QUEUE_DEPTH: usize = 4096;

pub const IPC_CMD_CREATE: usize = 0;
pub const IPC_CMD_SEND: usize = 1;
pub const IPC_CMD_RECV: usize = 2;
pub const IPC_CMD_CLOSE: usize = 3;
pub const IPC_CMD_DUP: usize = 4;
pub const IPC_CMD_BIND: usize = 5;
pub const IPC_CMD_CONNECT: usize = 6;
pub const IPC_CMD_ACCEPT: usize = 7;

const IPC_OK: isize = 0;
const IPC_EINVAL: isize = -22;
const IPC_EBADF: isize = -9;
const IPC_EAGAIN: isize = -11;
const IPC_EMFILE: isize = -24;
const IPC_EPIPE: isize = -32;
const IPC_ENOENT: isize = -2;
const IPC_EEXIST: isize = -17;
const IPC_EMSGSIZE: isize = -90;
const IPC_MAX_NAME_LEN: usize = 128;

pub type IpcHandle = Arc<Mutex<IpcHandleState>>;

pub struct IpcHandleState {
    object: IpcHandleObject,
}

enum IpcHandleObject {
    Channel {
        channel: Arc<Mutex<IpcChannel>>,
        side: usize,
    },
    Listener(Arc<Mutex<IpcListener>>),
}

type NamedEndpoint = (String, Arc<Mutex<IpcListener>>);

static NAMED_ENDPOINTS: Lazy<Mutex<Vec<NamedEndpoint>>> = Lazy::new(|| Mutex::new(Vec::new()));

struct IpcListener {
    refs: usize,
    pending: VecDeque<IpcHandle>,
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
        IPC_CMD_BIND => sys_bind(args),
        IPC_CMD_CONNECT => sys_connect(args),
        IPC_CMD_ACCEPT => sys_accept(args),
        _ => IPC_EINVAL,
    };
    args.rax = ret as u64;
}

fn sys_create(args: &mut ProcessContext) -> isize {
    let channel = Arc::new(Mutex::new(IpcChannel::new()));
    let handle0 = new_channel_handle(channel.clone(), 0);
    let handle1 = new_channel_handle(channel, 1);
    {
        let inner = handle0.lock();
        channel_from_handle_state(&inner).unwrap().lock().refs[0] += 1;
    }
    {
        let inner = handle1.lock();
        channel_from_handle_state(&inner).unwrap().lock().refs[1] += 1;
    }

    let slots = install_current_pair(handle0.clone(), handle1.clone());
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
            .inspect(|&slot| {
                task.ipc_handles[slot] = Some(duped.clone());
            })
    };
    let Some(slot) = slot else {
        close_handle_ref(duped);
        return IPC_EMFILE;
    };
    slot as isize
}

fn sys_bind(args: &mut ProcessContext) -> isize {
    let Some(name) = copy_name_arg(args.rsi as *const u8, args.rdx as usize) else {
        return IPC_EINVAL;
    };
    let name_key = name.clone();
    let listener = Arc::new(Mutex::new(IpcListener {
        refs: 1,
        pending: VecDeque::new(),
    }));
    let local = Arc::new(Mutex::new(IpcHandleState {
        object: IpcHandleObject::Listener(listener.clone()),
    }));

    {
        let mut named = NAMED_ENDPOINTS.lock();
        if named.iter().any(|(entry_name, _)| entry_name == &name) {
            drop(named);
            close_handle_ref(local);
            return IPC_EEXIST;
        }
        named.push((name, listener));
    }

    match install_current_handle(local) {
        Ok(slot) => slot as isize,
        Err(handle) => {
            unregister_listener_by_name(&name_key);
            close_handle_ref(handle);
            IPC_EMFILE
        }
    }
}

fn sys_connect(args: &mut ProcessContext) -> isize {
    let Some(name) = copy_name_arg(args.rsi as *const u8, args.rdx as usize) else {
        return IPC_EINVAL;
    };
    let listener = {
        let named = NAMED_ENDPOINTS.lock();
        let Some((_, listener)) = named.iter().find(|(entry_name, _)| entry_name == &name) else {
            return IPC_ENOENT;
        };
        listener.clone()
    };
    let channel = Arc::new(Mutex::new(IpcChannel::new()));
    let client = new_channel_handle(channel.clone(), 0);
    let server = new_channel_handle(channel, 1);
    {
        let inner = client.lock();
        channel_from_handle_state(&inner).unwrap().lock().refs[0] += 1;
    }
    {
        let inner = server.lock();
        channel_from_handle_state(&inner).unwrap().lock().refs[1] += 1;
    }

    match install_current_handle(client) {
        Ok(slot) => {
            let mut locked = listener.lock();
            locked.pending.push_back(server);
            slot as isize
        }
        Err(handle) => {
            close_handle_ref(handle);
            close_handle_ref(server);
            IPC_EMFILE
        }
    }
}

fn sys_accept(args: &mut ProcessContext) -> isize {
    let handle_id = args.rsi as usize;
    let Some(listener) = current_listener(handle_id) else {
        return IPC_EBADF;
    };
    let pending = {
        let mut locked = listener.lock();
        locked.pending.pop_front()
    };
    let Some(handle) = pending else {
        return IPC_EAGAIN;
    };
    match install_current_handle(handle) {
        Ok(slot) => slot as isize,
        Err(handle) => {
            let mut locked = listener.lock();
            locked.pending.push_front(handle);
            IPC_EMFILE
        }
    }
}

fn current_handle(handle_id: usize) -> Option<(Arc<Mutex<IpcChannel>>, usize)> {
    let handle = current_handle_ref(handle_id)?;
    let locked = handle.lock();
    match &locked.object {
        IpcHandleObject::Channel { channel, side } => Some((channel.clone(), *side)),
        IpcHandleObject::Listener(_) => None,
    }
}

fn current_listener(handle_id: usize) -> Option<Arc<Mutex<IpcListener>>> {
    let handle = current_handle_ref(handle_id)?;
    let locked = handle.lock();
    match &locked.object {
        IpcHandleObject::Channel { .. } => None,
        IpcHandleObject::Listener(listener) => Some(listener.clone()),
    }
}

fn install_current_pair(handle0: IpcHandle, handle1: IpcHandle) -> Option<(usize, usize)> {
    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut tasks = TASKS.lock();
    let task = tasks[current].as_mut().unwrap();
    let mut free = task
        .ipc_handles
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| entry.is_none().then_some(idx));
    match (free.next(), free.next()) {
        (Some(slot0), Some(slot1)) => {
            task.ipc_handles[slot0] = Some(handle0);
            task.ipc_handles[slot1] = Some(handle1);
            Some((slot0, slot1))
        }
        _ => None,
    }
}

fn install_current_handle(handle: IpcHandle) -> Result<usize, IpcHandle> {
    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut tasks = TASKS.lock();
    let task = tasks[current].as_mut().unwrap();
    let Some(slot) = task.ipc_handles.iter().position(Option::is_none) else {
        return Err(handle);
    };
    task.ipc_handles[slot] = Some(handle);
    Ok(slot)
}

fn copy_name_arg(ptr: *const u8, len: usize) -> Option<String> {
    if ptr.is_null() || len == 0 || len > IPC_MAX_NAME_LEN {
        return None;
    }
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    let name = core::str::from_utf8(bytes).ok()?.to_owned();
    if name.is_empty() { None } else { Some(name) }
}

fn current_handle_ref(handle_id: usize) -> Option<IpcHandle> {
    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    let tasks = TASKS.lock();
    let task = tasks[current].as_ref()?;
    task.ipc_handles.get(handle_id)?.as_ref().cloned()
}

fn dup_handle_ref(handle: &IpcHandle) -> IpcHandle {
    let locked = handle.lock();
    match &locked.object {
        IpcHandleObject::Channel { channel, side } => {
            channel.lock().refs[*side] += 1;
        }
        IpcHandleObject::Listener(listener) => {
            listener.lock().refs += 1;
        }
    }
    handle.clone()
}

fn close_handle_ref(handle: IpcHandle) {
    let locked = handle.lock();
    match &locked.object {
        IpcHandleObject::Channel { channel, side } => {
            let mut locked = channel.lock();
            if locked.refs[*side] != 0 {
                locked.refs[*side] -= 1;
                if locked.refs[*side] == 0 {
                    close_side(&mut locked, *side);
                }
            }
        }
        IpcHandleObject::Listener(listener) => {
            let mut locked = listener.lock();
            if locked.refs != 0 {
                locked.refs -= 1;
                if locked.refs == 0 {
                    let pending = core::mem::take(&mut locked.pending);
                    drop(locked);
                    unregister_listener(listener);
                    for pending_handle in pending {
                        close_handle_ref(pending_handle);
                    }
                }
            }
        }
    }
}

fn close_side(channel: &mut IpcChannel, side: usize) {
    let endpoint = &mut channel.endpoints[side];
    endpoint.closed = true;
    endpoint.queue.clear();
}

fn new_channel_handle(channel: Arc<Mutex<IpcChannel>>, side: usize) -> IpcHandle {
    Arc::new(Mutex::new(IpcHandleState {
        object: IpcHandleObject::Channel { channel, side },
    }))
}

fn channel_from_handle_state(handle: &IpcHandleState) -> Option<&Arc<Mutex<IpcChannel>>> {
    match &handle.object {
        IpcHandleObject::Channel { channel, .. } => Some(channel),
        IpcHandleObject::Listener(_) => None,
    }
}

fn unregister_listener(listener: &Arc<Mutex<IpcListener>>) {
    let mut named = NAMED_ENDPOINTS.lock();
    if let Some(idx) = named
        .iter()
        .position(|(_, entry_listener)| Arc::ptr_eq(entry_listener, listener))
    {
        named.swap_remove(idx);
    }
}

fn unregister_listener_by_name(name: &str) {
    let mut named = NAMED_ENDPOINTS.lock();
    if let Some(idx) = named.iter().position(|(entry_name, _)| entry_name == name) {
        named.swap_remove(idx);
    }
}
