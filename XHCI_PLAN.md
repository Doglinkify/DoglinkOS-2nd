# XHCI 驱动引入计划

## 目标与边界

首期在 x86_64/UEFI 启动的 DoglinkOS-2nd 上增加 PCIe xHCI 主机控制器驱动，支持直连到根集线器端口的 USB 2.0 HID Boot 键盘和鼠标。键盘输入进入现有 TTY/`INPUT_BUFFER` 路径；鼠标至少保留当前 PS/2 驱动已有的滚动行为。驱动要能同时和现有 PS/2 输入共存，且没有 xHCI、没有设备或初始化失败时，系统必须继续正常启动。

首期不包含：USB 存储、USB 3.x SuperSpeed 数据路径、外接 Hub、等时传输、HID Report Descriptor 的通用解释器、热插拔后的资源回收、IOMMU 支持和多控制器负载均衡。驱动内部仍须按 xHCI 的 64 位 DMA 要求实现，不能以 QEMU 恰好可用的布局替代规范要求。

`builder` 已经提供可重复的验收设备：`--ps2-special 2` 会创建 `qemu-xhci` 和 `usb-kbd`，`--ps2-special 3` 会创建 `qemu-xhci` 和 `usb-mouse`。首期以该组合为持续验收基线；最终合入前需在至少一台真实 xHCI 主机上复测。

## 交付原则

每个提交必须：

1. 只完成一个可描述的行为增量，不能包含顺手重构、格式化无关文件或后续提交才需要的接口。
2. 在该提交本身可构建、可启动；未接入启动路径的库代码不改变现有行为，接入后的代码要在硬件缺失和失败时降级而非 panic。
3. 通过公共静态门槛：`cargo fmt --all -- --check` 与 `cargo check -p DoglinkOS-2nd --target x86_64-unknown-none`。任何纯逻辑模块还要随提交提供 host 可运行的单元测试。
4. 功能提交使用对应的 QEMU 命令验收，并把串口日志中的成功、无设备和超时/失败三种情形写入该提交说明。启动命令基线为：

   ```bash
   cargo run -p builder -- --boot --ps2-special 2 --serial stdio
   cargo run -p builder -- --boot --ps2-special 3 --serial stdio
   ```

5. 每个提交在提交前从干净工作树执行上述所需检查；提交信息采用 `xhci:` 前缀。除最终文档提交外，不把生成的 `DoglinkOS-2nd.img`、`initrd.img` 或 `target/` 纳入 Git。

当前基线已通过两项公共静态门槛。运行 QEMU 时，验收者需在图形窗口聚焦后输入字符或产生鼠标滚动，并从串口确认内核没有 fault、没有超时循环和没有重复枚举。

## 设计约束

### 控制器和内存

- PCI 识别条件为 class `0x0c`、subclass `0x03`、programming interface `0x30`；读取 BAR0 时必须支持 64 位 Memory BAR，拒绝 I/O BAR 和长度不足的映射。
- 所有设备寄存器通过 volatile 访问；MMIO 映射使用 `PRESENT | WRITABLE | NO_CACHE`，按页向下/向上取整。寄存器偏移来自 `CAPLENGTH`、`DBOFF`、`RTSOFF`，不得写死 QEMU 的偏移。
- 控制器初始化顺序为：启用 PCI Memory Space 与 Bus Master -> BIOS/OS ownership handoff（有能力时）-> 停止并等待 `HCHalted` -> 主机控制器复位并等待 `CNR` 清除 -> 分配/清零 DMA 区 -> 设置 `DCBAAP`、`CRCR`、`ERST*`、`CONFIG.MaxSlotsEn` -> 启动并等待运行。每一个等待都使用统一的、有诊断信息的有限轮询超时。
- DMA 分配器返回物理地址、HHDM 虚拟地址、页数和对齐保证；失败返回 `Result`。DCBAA、Scratchpad 指针数组、上下文、TRB 环和事件环都按规范所需的 16/64 字节对齐并在提交给硬件前清零。没有 IOMMU 的首期假设必须在代码和文档中明确。
- TRB 环保留 Link TRB，维护 producer/consumer index 与 cycle bit；命令、控制、interrupt IN 和事件分别拥有清晰的所有权。事件环在消费事件后才更新 `ERDP`；不能重用尚未完成的 TRB 或 DMA 缓冲区。
- 初始化、控制传输和端点传输的 `Result` 必须保留 completion code、slot/endpoint 和超时上下文。初始化失败要停用该控制器并释放尚未交给硬件的资源，不能让 `unwrap()`、无限循环或半初始化的全局对象进入启动路径。

### USB 与输入

- 从 Supported Protocol Extended Capability 建立“物理端口 -> 协议/速度”映射，扫描 `PORTSC.CCS` 变化；首期只调度 USB 2.0 根端口。连接后执行端口复位、Enable Slot、Address Device、读 Device/Configuration Descriptor、Set Configuration、Configure Endpoint。
- 描述符遍历必须按 `bLength` 前进并验证剩余长度，拒绝零长度、越界及截断的描述符。仅接受 interface class `0x03`、subclass `0x01`、protocol `0x01`（键盘）或 `0x02`（鼠标），以及一个 IN interrupt endpoint；其余设备记录原因后跳过，不影响其他端口。
- HID Boot 键盘以按下位图进行差分，映射 HID usage 到现有终端消费的 Set-1 扫描码，再生成 make/break。鼠标将 Boot report 翻译为公共的鼠标语义接口，不能伪造 PS/2 原始包或依赖固定报告长度之外的数据。
- 先采用 xHCI 事件环轮询：第 10 步把当前 `main.rs` 的内核空闲路径从忙等改为“有界 `xhci::poll()` + `hlt`”，一次处理有界数量事件并重新提交 interrupt IN TRB。不能在定时器 IDT 上下文解析传输或分配内存。完成稳定闭环后再引入 MSI/MSI-X；中断处理只确认和转移事件，绝不在 IDT 上下文等待命令或打印大量日志。

## 建议的模块边界

```text
kernel/src/
  pcie/                 BDF、ECAM、BAR、Command/capability 访问
  mm/dma.rs             连续物理 DMA 缓冲、对齐、清零、生命周期
  xhci/
    mod.rs              控制器列表、init()/poll()、错误和日志边界
    regs.rs             capability/operational/runtime/doorbell 的 volatile 封装
    trb.rs              #[repr(C, align(16))] TRB、completion code、纯逻辑测试
    ring.rs             command/transfer/event 环和 cycle bit
    context.rs          slot/endpoint/input context，依据 CSZ 选择大小
    controller.rs       reset、DCBAA、命令完成、端口和 slot 生命周期
    usb.rs              请求、描述符的受检解析、枚举状态机
    hid.rs              Boot 键盘/鼠标 report 差分与输入事件
  inputdev.rs           公共 submit_keyboard_scancode()/submit_mouse_*() 入口
  int.rs, apic/         仅在最后的 MSI/MSI-X 提交中增加向量注册和 EOI
```

`xhci` 不依赖 VFS、任务或终端实现；它只向 `inputdev` 提交规范化事件。`inputdev` 的公共入口包含现有 PS/2 的终端回显和 `INPUT_BUFFER` 路径，因此输入源不会各自复制这段逻辑。

## 原子提交序列

### 1. `docs: add XHCI delivery plan`

仅加入本文档，冻结首期范围、验收矩阵和提交拆分。该提交本身不改内核行为；运行公共静态门槛确认文档提交没有附带构建回归。

### 2. `pcie: provide typed BDF, BAR and command access`

将当前只读、`repr(C)` 的 PCI 配置空间访问演进为小型 typed API：`Bdf`、设备信息、volatile config read/write、Memory BAR 解码、command bit 更新和 capability 链遍历。修复 ECAM 地址对 MCFG 非零 `bus_range.start` 的偏移计算；枚举回调继续兼容现有 AHCI/NVMe 调用者。添加无硬件单元测试覆盖 BDF/ECAM 地址、32/64 位 BAR 和 capability 链循环/越界拒绝。

验收：公共静态门槛；无 xHCI 的常规 QEMU 启动仍枚举原有 AHCI/NVMe 且无新增设备。这个提交不探测或启用 xHCI。

### 3. `mm: add fallible aligned DMA and MMIO mappings`

新增 `mm::dma` 和最小 MMIO 映射封装，集中连续页分配、物理/虚拟转换、对齐、清零、页数记录和回收；为 BAR 映射提供页对齐及 `NO_CACHE` 标志设置。补足 `find_continuous_mem` 的失败语义，避免把物理地址 0 误当作成功。此提交只提供通用能力，不迁移 AHCI/NVMe。

验收：DMA 布局、对齐、页数计算单元测试；公共静态门槛；普通 QEMU 启动。检查失败路径不会改动页分配位图。

### 4. `xhci: add register layout, TRBs and ring tests`

创建未接入启动流程的 `xhci` 库骨架：寄存器偏移封装、TRB 常量与 `repr(C, align(16))` 布局、completion code，以及 command/transfer/event 环的纯内存实现。测试包括 Link TRB 放置、环回时 cycle bit 翻转、满环拒绝、事件消费者推进和 TRB 字节布局；用编译期或测试断言锁定大小与对齐。

验收：新增单元测试和公共静态门槛。该提交不触碰 PCI，也不访问硬件，因此现有启动行为不变。

### 5. `xhci: discover and safely reset controllers`

实现 `xhci::init()` 的发现与受控复位：按 class/subclass/prog-if 查找设备，启用 PCI command 的 memory/bus-master 位，映射 BAR0，读取 capability，执行可选 BIOS ownership handoff，并按有限超时停机、复位、等待 `CNR`。用 `ControllerState` 表示 `Discovered`、`Reset`、`Failed`，失败只打印一个带 BDF 和阶段的告警并继续启动。

验收：使用 `--ps2-special 2` 启动时日志报告 xHCI BDF、版本、端口数和 reset 成功；在未附加 xHCI 的启动中显示零控制器且正常启动。故意缩短测试超时或在 mock 寄存器中置位 `CNR` 的单元测试覆盖失败退出，不允许无限等待。

### 6. `xhci: initialize DMA contexts and command/event rings`

在已复位的控制器中分配 DCBAA、必要的 scratchpad 数组、命令环和事件环，设置 `DCBAAP`、`CRCR`、`ERSTSZ`、`ERSTBA`、`ERDP` 与 `CONFIG.MaxSlotsEn` 后启动控制器。实现 doorbell 0、命令 completion 的事件匹配和超时；保持 interrupter 禁用，只由显式轮询消费事件。控制器启动失败回退到 `Failed`。

验收：QEMU xHCI 启动日志显示 running、MaxSlots、context size、scratchpad count 和第一条 command completion；重复启动三次均无页分配告警。静态测试覆盖物理地址写入寄存器和命令 completion 匹配。

### 7. `xhci: enumerate USB 2 root-port devices`

实现 Supported Protocol capability 解析、根端口连接检测和 USB 2.0 端口复位。对已连接的端口完成 Enable Slot、Address Device、Endpoint 0 最大包长更新和读取前 8 字节 Device Descriptor；将 slot、port、speed 与状态保存在控制器中。未连接、USB 3.x 或命令失败的端口单独记录并跳过。

验收：`usb-kbd` 和 `usb-mouse` 分别显示端口号、slot id、speed、VID:PID；没有 USB 设备时没有命令超时。端口状态机单测覆盖连接、复位失败、地址失败和重复扫描不重复分配 slot。

### 8. `xhci: parse descriptors and configure HID Boot endpoints`

实现受检 USB descriptor parser 与标准 control request。读取完整 Device/Configuration Descriptor，选中唯一支持的 HID Boot interface，发出 `SET_CONFIGURATION`、必要的 `SET_IDLE`，创建 input context 并通过 Configure Endpoint 配置 IN interrupt endpoint。将 endpoint 的 interval、最大包长、DCS 和传输环记录下来；不支持的接口保留诊断但不失败整个控制器。

验收：解析器单测覆盖多个 interface/alternate setting、截断、`bLength == 0`、错误 endpoint 和非 HID 设备；QEMU 键盘/鼠标日志显示已配置的 slot、endpoint 和 interval。此时不提交输入到终端，但端点配置必须完成且启动稳定。

### 9. `input: expose source-neutral keyboard and mouse submission`

从 `inputdev` 中抽取现有终端回显、TTY 和 `INPUT_BUFFER` 写入逻辑，新增 source-neutral 的键盘扫描码与鼠标语义提交入口。PS/2 保持原有调用顺序和行为，仅改为调用这些入口；为键盘 make/break、TTY 开关、串口回显和鼠标滚动添加纯逻辑测试或可替换 sink 测试。

验收：不启用 xHCI 的 QEMU/实体机 PS/2 键盘和鼠标回归通过；公共静态门槛。该提交还不启用 USB 输入，因此可以独立回滚或 bisect。

### 10. `xhci: deliver HID Boot reports by polling`

实现键盘和鼠标 Boot report 解析、按键状态差分、HID usage 到 Set-1 映射、鼠标按钮/位移/滚轮转换；为每个已配置 interrupt IN endpoint 提交初始 TRB。`xhci::poll()` 有界地消费事件、校验 completion code、提交输入、重新排队同一 DMA 缓冲。将它接入 `main.rs` 的内核空闲路径，并以 `hlt` 取代该路径的纯忙等；保证在无控制器时是低成本 no-op，且不改变 `ps2_poll` 调试路径。

验收：`--ps2-special 2` 下按字母、Shift、Enter、Backspace 和释放键能在 TTY 正确工作，持续输入不丢失/重复；`--ps2-special 3` 下滚轮行为与 PS/2 一致。串口日志无 transfer timeout、ring overflow 或页错误；无 xHCI、无 USB 设备和 PS/2-only 三种启动路径都回归通过。

### 11. `xhci: route event interrupts through MSI or MSI-X`

在轮询闭环稳定后，增加 PCI capability 中 MSI/MSI-X 的受检配置、可分配 IDT 向量及共享的 xHCI interrupt handler。处理函数确认 `USBSTS`/`IMAN`、标记待处理控制器并发送 LAPIC EOI；实际事件解析仍由 `xhci::poll()` 在非中断上下文完成。MSI/MSI-X 配置失败时回退到第 10 步的轮询模式，不能失去输入。

验收：QEMU 键盘和鼠标在 MSI/MSI-X 模式下工作，日志显示一次向量配置及事件计数；强制 capability 缺失时明确回退轮询仍可输入。验证 IDT 既有 timer、异常和 PS/2 向量不被覆盖，并在真实机复测至少一个控制器。

### 12. `docs: record XHCI support and validation matrix`

更新 README 或新增驱动文档，记录支持范围、已验证的 QEMU 与实体硬件、启动命令、已知限制、故障日志所需字段和回归步骤。该提交只在前述功能提交均已合入且测试结果可复现后创建。

验收：公共静态门槛、两条 QEMU 命令、PS/2 回归和至少一份真实机结果均在提交说明或 CI 工件中可追溯。

## 合并与回归要求

提交顺序不可压缩：第 2--3 步解决所有驱动共享的硬件访问前提，第 4 步先锁定易错的数据结构，第 5--8 步形成枚举闭环，第 9--10 步才让外设影响用户输入，第 11 步仅优化事件通知方式。每个提交都能被单独 `git bisect`、构建和启动；不接受“先放一个不能启动的半成品，再由下一提交修复”的历史。

在合并第 10 步及以后提交前，最少执行：无 xHCI 启动、xHCI 无设备、QEMU USB 键盘、QEMU USB 鼠标、PS/2 键盘/鼠标、重复冷启动三次，以及实体机冷启动。所有超时必须带 BDF、端口/slot/endpoint、阶段和 completion code；任何内核 panic、无限轮询、DMA 对齐错误、重复输入或已有存储/PS2 回归都阻断合并。
