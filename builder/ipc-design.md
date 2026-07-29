# IPC 设计草案

## 目标

- 支持进程间消息传递
- 支持阻塞/唤醒
- 尽量复用现有 `TASKS`、`waitpid` 和调度器结构
- 先做“可用且简单”的版本，再扩展共享内存/信号量

## 现状约束

- 当前只有 `waiting_pid`，只能等子进程退出
- 调度器在 `kernel/src/task/sched.rs` 只按任务可运行性和时间片调度
- 进程表在 `kernel/src/task/process.rs`，适合直接挂 IPC 状态
- 用户态 syscall 已经按寄存器传参，适合新增少量 IPC syscall

## 方案

### 1. 核心对象

- `IpcEndpoint`：单向消息端点，类似 mailbox
- `IpcChannel`：一对端点，A->B 和 B->A
- `IpcMessage`：固定头 + 可变载荷

### 2. 进程状态扩展

给 `Process` 增加：

- `state: ProcessState`
- `wait_reason: WaitReason`
- `ipc_handles: [Option<Arc<Mutex<IpcEndpoint>>>; N]`

`WaitReason` 至少包含：

- `None`
- `WaitPid(usize)`
- `IpcRecv(usize)`
- `IpcSend(usize)`

调度器只运行 `Runnable`，其余都跳过。

### 3. 同步语义

- `send`：
  - 目标队列未满，直接入队
  - 目标进程若在等该端点，唤醒它
  - 队列满时，按 flag 决定阻塞或返回 `EAGAIN`
- `recv`：
  - 队列非空，直接取消息
  - 队列空时，按 flag 决定阻塞或返回 `EAGAIN`

### 4. 传递方式

第一版只做：

- 字节消息
- 固定最大长度（例如 4 KiB）
- 内核拷贝进出

后续再加：

- 零拷贝共享内存
- 句柄传递
- `select/poll` 风格等待多个端点

### 5. syscall 建议

只新增一个 syscall：

- `ipc(cmd, arg0, arg1, arg2, arg3, arg4) -> isize`

其中 `cmd` 区分功能，类似 `ioctl`。建议命令集：

- `IPC_CREATE`
- `IPC_SEND`
- `IPC_RECV`
- `IPC_CLOSE`
- `IPC_BIND`
- `IPC_CONNECT`
- `IPC_DUP`

建议先选“匿名 channel + fork 继承”作为最小可用集。

## 推荐落地顺序

1. 增加 `ProcessState` / `WaitReason`
2. 把 `waitpid` 改成统一阻塞框架
3. 实现匿名 `IpcChannel`
4. 增加 `ipc_send/recv`
5. 再补命名端点和共享内存

## 风险点

- 现有调度器和 `TASKS` 锁粒度较粗，IPC 频繁唤醒时要避免长时间持锁
- syscall 里直接拷贝用户缓冲区，必须先做地址合法性检查
- 需要定义清晰的资源回收：进程退出时自动关闭其 IPC 句柄
