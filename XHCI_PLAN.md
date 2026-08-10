# XHCI 第二阶段驱动计划：热插拔与 USB 大容量存储

## 1. 目标、基线与范围

本轮以现有 xHCI USB 2.0 HID Boot 键盘/鼠标实现为基线，补齐两个可独立验收的能力：

1. 根端口 USB 2.0 设备热插拔，包括连接、断开、同端口再次连接和资源回收。
2. USB Mass Storage Class（MSC）设备的大容量存储访问，并作为可枚举的块设备暴露给现有 VFS。

首期存储传输协议固定为 **Bulk-Only Transport (BOT, BBB)**，设备类别为 Mass Storage / SCSI Transparent Command Set（class `0x08`、subclass `0x06`、protocol `0x50`）。支持 USB 2.0 直连根端口、单 LUN、512 字节逻辑块、只读数据路径。`READ CAPACITY(10)` 与 `READ(10)` 是本轮必须项；`WRITE(10)`、4 KiB 逻辑块、多个 LUN、UAS、USB 3.x SuperSpeed、外接 Hub、Suspend/Resume、IOMMU 和从 USB 盘启动不在本轮范围内。

当前实现的限制是：根端口仅在控制器启动后扫描一次，运行时事件循环只处理 HID Transfer Event；Port Status Change Event 被确认后丢弃，断开后也不会 Disable Slot。因此本轮不把“反复扫描所有端口”当作热插拔方案，而以 xHCI 的端口状态变化事件驱动生命周期。

## 2. 完成定义

完成时应满足：

- 在系统启动后插入 USB 键盘、鼠标或 BOT U 盘，设备在有限时间内完成枚举；拔出后不再保留 slot、端点 ring、DMA 缓冲或 `/dev` 条目；再次插入可重新工作。
- HID 设备断开后不会持续重排 interrupt IN，也不会产生无限错误日志；同端口更换为另一 HID 设备能重新提交输入。
- 合格的 BOT 存储设备出现为 `/dev/usbN`，可读取容量和扇区数据；移除后新打开失败，已打开的句柄后续 I/O 返回明确的设备已移除错误，绝不访问已释放 DMA 内存。
- 无 xHCI、无 USB 设备、非 HID/非 MSC 设备、枚举或 BOT 失败都只影响该端口，既有 AHCI、NVMe、PS/2、TTY 和 initrd 启动路径保持可用。
- 所有等待、命令、传输和恢复流程都有边界；IDT/MSI 路径只确认中断和设置待处理状态，不分配内存、不发送命令、不打印大量日志。

## 3. 架构与生命周期

```text
Port Status Change Event / 定期兜底扫描
             |
             v
  PortManager: Debounce -> Reset -> Enable Slot -> Address -> Describe
             |                                      |
             |                               DeviceKind 判别
             |                         +------------+------------+
             v                         v                         v
       Disconnect                HID interrupt IN          MSC bulk IN/OUT
             |                         |                         |
             v                         v                         v
 Disable Slot / 回收 DMA       inputdev 提交             BOT + SCSI + BlockDevice
             |                                                   |
             +------------------------> /dev 设备注册 <----------+
```

### 3.1 端口状态机

每个受支持的物理 USB 2.0 根端口都拥有独立 `PortRecord`：协议映射、最新 `PORTSC`、代数（generation）、状态、可选 `DeviceHandle` 与延迟重试截止时间。状态至少包括 `Disconnected`、`Debouncing`、`Resetting`、`Enumerating`、`Active`、`Removing`、`Failed`。

- 收到 Port Status Change Event 后读取并 W1C 清除该端口所有已知 change bits，使用 `CCS` 作为连接真值；不能将事件本身视为连接成功。
- 连接变化进入 `Debouncing`，以有限轮询确认稳定的 `CCS` 后再复位。复位、枚举和配置的每一阶段都再次检查该端口的 generation 与 `CCS`；已断开的异步完成结果必须丢弃。
- 从 `Active` 断开时，先停止软件提交，再取消/忽略该设备尚未完成的 transfer event，发送 `Disable Slot`，清空 DCBAA slot 指针，解除 `/dev` 注册，最后释放设备的 DMA/ring/context。若控制器不再响应，隔离并停用整个 controller，不能释放控制器仍可能 DMA 的内存。
- `Disable Slot` 失败或超时保留资源并标记 controller 不健康，不可带着不确定 DMA 所有权继续重用 slot。
- `Failed` 不是终态：仅在新的连接变化或受限退避计时器到期后允许重新尝试。连续失败使用指数退避，并在日志中包含 BDF、端口、generation、阶段和 completion code。

### 3.2 事件与并发模型

扩展 event ring 解码，区分 Command Completion、Transfer Event 和 Port Status Change Event。`xhci::poll()` 是唯一能改变 `PortRecord`、分配/释放 DMA、访问 VFS 设备表或发 xHCI command 的上下文。MSI handler 只确认 `USBSTS`/`IMAN` 并标记 controller；轮询路径以固定预算处理事件，并在每轮处理有限数量的待枚举/待移除端口，避免一个损坏设备饿死 HID。

运行时不再以 `Vec<RootDevice>` 隐式表示端口状态。改为稳定的端口表和带 `slot_id`、`generation`、`kind` 的设备对象；任何 transfer event 均必须匹配 slot、endpoint、TRB 地址和 generation。未知、过期或断开后的事件仅确认并计数，不可重排传输。

### 3.3 枚举与类驱动边界

`usb.rs` 负责标准请求、描述符受检遍历和不依赖硬件的枚举状态；`controller.rs` 负责 xHCI contexts、ring、doorbell 和端口/slot 生命周期。描述符解析返回通用 `InterfaceDescriptor`/`EndpointDescriptor`，由 HID 和 MSC 类驱动分别选择接口，不能再将配置描述符直接限定为 HID。

BOT 驱动放在 `kernel/src/xhci/msc.rs`，只依赖一个受控的 bulk transfer 抽象；块设备适配层放在 `kernel/src/blockdev/usb.rs`。BOT/SCSI 解析和 CBW/CSW 字节布局必须可在 host 单元测试，不直接依赖 MMIO。

## 4. 存储协议要求

### 4.1 BOT 事务

每个事务严格执行 `CBW -> 可选数据阶段 -> CSW`，一次只允许一个未完成命令。CBW 使用固定 31 字节布局、唯一递增 tag、正确 data direction 和 transfer length；CSW 必须是 13 字节、匹配签名/tag，并检查 residue 与 status。

- 初始探测顺序：`GET_MAX_LUN`（stall 等同于 LUN 0）-> `INQUIRY` -> `TEST UNIT READY`（有限重试）-> `REQUEST SENSE`（失败诊断）-> `READ CAPACITY(10)`。
- 读取路径为 `READ(10)`；每次传输不得超过类驱动配置的 DMA bounce buffer 上限，并按控制器/端点最大传输长度分段。CBW、数据和 CSW 各自有独立 completion 校验。
- 对 CSW phase error、tag/signature 不匹配、bulk stall、传输错误或超时，执行 BOT Reset class request 和两个 bulk endpoint 的 `CLEAR_FEATURE(ENDPOINT_HALT)`，然后有限重试当前命令。恢复失败使该设备离线，不影响其他端口。
- 仅接受 `READ CAPACITY(10)` 的 512-byte block length；容量字段为 `0xffff_ffff` 时说明需要 READ CAPACITY(16)，本轮明确拒绝并记录原因。写入请求返回只读错误，不伪装成功。

### 4.2 块设备与 VFS 契约

定义共享的内部块设备接口，至少包含逻辑块大小、块数、`read_blocks(lba, buffer)`、online 状态和稳定设备 ID。USB 设备以该接口接入，而不是复制 AHCI/NVMe 的 `fatfs::Read` 逻辑。

- 为 `/dev/usbN` 增加 devfs 枚举和打开路径；编号按 controller/BDF、port、generation 或单调 ID 稳定生成，移除后不可把仍被打开的旧句柄重新指向新设备。
- 首轮只需提供 raw 块设备读取。分区解析、FAT 挂载和 syscall mount 只有在抽取出不绑定 AHCI/NVMe 的通用 `BlockIo` 后才接入；USB 移除时已有 mount 必须变为 I/O error，不能 panic 或 use-after-free。
- 设备表使用可引用的 online object：devfs 打开取得强引用，热拔出原子地置 offline 并撤销新发现；对象最后一个句柄释放后才回收软件资源。xHCI 的 DMA 资源仍按端口移除序列处理。

## 5. 原子提交计划

### 1. `docs: replace xhci plan for hotplug and mass storage`

提交本文档，冻结本轮范围、BOT-only 决策、热拔出语义和验收矩阵。无行为改动。

### 2. `xhci: decode port status events and model root ports`

补充 Port Status Change Event TRB、完整 `PORTSC` change-bit 常量和纯逻辑 event decoder；将根端口协议映射保存在 controller 中，建立可测试的 `PortRecord` 状态机。保留当前启动时枚举行为，但不接入运行时重配置。

验收：单测覆盖 connect/disconnect/change-bit acknowledge、过期 generation 和失败退避；`cargo fmt --all -- --check`、`cargo check -p DoglinkOS-2nd --target x86_64-unknown-none`。

### 3. `xhci: handle root-port connect and disconnect`

在 `poll()` 中处理端口状态变化，完成有限去抖、重置、枚举、`Disable Slot` 和 DMA 所有权回收。将 HID 的 transfer 错误改为根据端口 generation 丢弃，而非无条件重排。增加低频受限兜底扫描，覆盖可能丢失的 change event，但不替代事件驱动。

验收：QEMU 启动后使用 QMP `device_add`/`device_del` 插拔 `usb-kbd`、`usb-mouse`；各进行三轮插入、输入、拔出、重新插入。日志必须显示一次 add/remove 和 slot 回收，无重复枚举、transfer timeout 循环或 ring overflow。

### 4. `xhci: generalize configuration descriptor selection`

把 HID 专用配置描述符选择重构为通用受检遍历器，保留 HID Boot 行为和错误日志；新增 MSC BOT 接口选择结果，验证 bulk IN/OUT 成对、非零 max packet 和 endpoint attributes。

验收：host 单测覆盖混合 HID/MSC、多 interface、alternate setting、截断和 malformed `bLength`；现有 USB HID QEMU 验收完全回归。

### 5. `xhci: add bulk endpoint transfer primitives`

实现 bulk IN/OUT transfer ring、单 TD completion 匹配、短包规则、端点 halt 诊断和有界等待。与 HID interrupt ring 分离，任何一个设备只能拥有自己的 ring/DMA 缓冲。

验收：ring 与 completion 单测；QEMU 下完成一个受控 bulk descriptor/探测传输，不影响同时连接的 HID。

### 6. `xhci: implement USB mass storage BOT transport`

新增 `msc.rs`：CBW/CSW、GET_MAX_LUN、INQUIRY、TEST UNIT READY、REQUEST SENSE、READ CAPACITY(10)、READ(10)，以及 BOT reset/clear-halt 恢复。所有整数均按小端编码，读请求分块且有 DMA 上限。

验收：host 单测覆盖 CBW/CSW 布局、tag/residue/status、SCSI CDB 编码、stall 恢复决策和容量边界；QEMU `usb-storage,drive=...` 显示 vendor/product、容量、块大小和 LUN 0。

### 7. `blockdev: register read-only USB storage devices`

新增 USB block-device manager 与 `/dev/usbN` devfs 文件，接入通用块设备接口，实现 offset 到 LBA 的受检读取、跨扇区读取和离线错误；不在此提交支持挂载。

验收：串口或内核测试读取 MBR/GPT 首扇区并比对测试镜像内容；`/dev` 列表出现设备，移除后入口消失，旧句柄读返回错误。

### 8. `blockdev: mount USB storage partitions safely`

抽取 AHCI/NVMe 分区和 mount 所需的通用 `BlockIo`，为 USB 增加分区与 FAT 只读挂载路径。所有 `unwrap()` 热路径改为可传播的错误；移除已挂载设备后的 read/seek 返回 I/O error。

验收：在 FAT 测试 U 盘上读取已知文件、校验内容；设备移除后访问和卸载不 panic；AHCI、NVMe 和 initrd 的既有挂载测试回归。

### 9. `builder: add repeatable USB hotplug and storage validation`

为 builder/QEMU 提供 USB 存储镜像和 QMP 热插拔测试入口，文档化设备 ID、QMP 命令和串口断言。不得依赖手工图形窗口才能验证存储或断开路径。

验收：CI 或可重复脚本覆盖无设备、启动时已连接、运行时插入、运行时拔出、重新插入、BOT reset 失败和两个设备并存。

### 10. `docs: record xhci hotplug and storage support`

更新 README 与支持矩阵，列出已验证 controller、设备、QEMU 版本、限制、日志字段和故障收集方法。

## 6. 验收矩阵与阻断条件

每个功能提交必须通过格式化和 kernel target check；涉及纯逻辑协议的提交还必须提供 host 单元测试。最终合并至少验证：

| 场景 | 预期 |
| --- | --- |
| 无 xHCI / 无 USB 设备 | 正常启动，无资源泄漏和长超时 |
| HID 启动时连接、运行时插拔 | 输入恢复，slot 正确回收，不重复按键 |
| BOT U 盘启动时连接 | `/dev/usbN` 可读，容量与内容正确 |
| BOT U 盘运行时插拔 | 新句柄与 devfs 条目正确更新，旧句柄返回设备已移除 |
| HID 与 U 盘同时连接 | 输入和存储并行正常，无 event/ring 串扰 |
| 非法描述符、非 BOT MSC、bulk stall | 端口失败可诊断，不影响其他设备 |
| AHCI、NVMe、PS/2、initrd | 既有启动、读取和挂载路径无回归 |

下列任一情况阻断合并：在 IDT 中等待或分配、热拔出后的 DMA use-after-free、slot 泄漏、无限重试/日志风暴、将旧 `/dev/usbN` 句柄重绑定到新设备、BOT CSW 未验证即返回数据，或任一既有存储/输入回归。

## 7. 真实硬件验证

QEMU 仅作为回归基线。第 3、6、8、9 步完成后，分别在至少一台真实 xHCI 主机和两个不同的 USB 2.0 存储设备上验证冷启动、热插、热拔、重新插入与长时间顺序读取。记录 controller BDF、xHCI 版本、设备 VID:PID、协议/速度、slot、端点、完成码和失败恢复日志；遇到 controller 在断开时停止响应，保留资源并报告，不以强制释放掩盖 DMA 安全问题。
