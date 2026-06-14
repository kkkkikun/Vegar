### **StarryOS io_uring 性能登顶与架构演进宏观计划**

---

### **第一阶段：专业级稳健性与标准基准对齐（当前阶段的升华）**

**目标：** 将 M0-M3 的“实验室代码”转化为“工业级代码”，并引入专业测试框架进行客观背书。

*   **1. 整合 OpenAnolis `perf-test-for-io_uring`**
    *   **任务：** 移植或适配 Anolis 仓库中的 `round_read` 和 `round_write` 测试单元。由于这些测试基于 `liburing`，你需要为 StarryOS 实现一个极简的静态 `liburing` 垫片（Shim）。
    *   **价值：** 证明你的 `io_uring` 实现不是“自圆其说”，而是能通过第三方专业框架的严苛压测，特别是高并发 `io_depth` 下的稳定性。
*   **2. 消除 M0-M3 遗留风险**
    *   **任务：** 彻底清理 `PinnedUserBuf` 在高并发下的内存生命周期问题（Pinning 机制）。确保在异步 I/O 进行时，用户态 `munmap` 不会导致内核崩溃。 [Anolis io_uring 性能测试框架](https://github.com/OpenAnolis/perf-test-for-io_uring)

---

### **第二阶段：初赛指标针对性“刷分”攻坚（提分核心期）**

**目标：** 通过 `io_uring` 改造或透明注入，提升 [初赛测评指标.md](初赛测评指标.md) 中关键项目的得分。

*   **1. I/O 性能突破（针对 Iozone）**
    *   **策略：** 传统 `Iozone` 使用同步 `read/write`。你可以尝试提供一个 **`LD_PRELOAD` 钩子** 或修改库函数，将 `Iozone` 的大块 I/O 读写透明重定向到 `io_uring`。
    *   **提分点：** 在 StarryOS 的 VFS 层实现 **预取（Prefetch）**。当 `io_uring` 收到一个 `READ` SQE 时，内核 worker 不仅读取当前请求，还异步预读后续块，从而在 `Iozone` 的顺序读测试中跑出惊人数据。
*   **2. 网络性能飞跃（针对 Iperf / Netperf）**
    *   **策略：** 改造 `axnet` 协议栈，实现 `IORING_OP_SEND` 和 `IORING_OP_RECV`。
    *   **提分点：** 使用 `IORING_REGISTER_BUFFERS`（你在 M3 已实现）。通过固定缓冲区减少网络包在内核与用户态之间的多次拷贝。配合 `io_uring-echo-server` 验证在高并发小包场景下的 CPU 占用率降低。
*   **3. 系统调用延迟优化（针对 Lmbench）**
    *   **策略：** 改造 `Lmbench` 中的 `lat_syscall` 测试。
    *   **提分点：** 对比单次 `ecall` vs `io_uring_enter` 批量提交 NOP。利用 `io_uring` 的批量化特性，在测试“平均每个 syscall 耗时”时，将成本压缩到极致。 [Anolis io_uring Echo Server](https://github.com/OpenAnolis/io_uring-echo-server)

---

### **第三阶段：架构通用化与异步 Trait 重构（架构领先期）**

**目标：** 参考 Redox OS 的 `AsyncScheme`，实现你目标中的“驱动跨 OS 复用”。

*   **1. 实现内核级 `AsyncRead / AsyncWrite` Trait**
    *   **任务：** 重构 `IoRing` 的 worker 逻辑。不再硬编码针对 `Pipe` 或 `File` 的处理，而是让 worker 调用通用的 `poll_handle` 接口。
    *   **价值：** 这是通用性的终极体现。只要底层驱动（无论是虚拟串口还是物理网卡）满足了异步 Trait，`io_uring` 就能瞬间支持该设备。
*   **2. 优化其他阻塞式系统调用**
    *   **任务：** 将 `stat`, `mkdir`, `unlink` 等 VFS 元数据操作纳入 `io_uring`。
    *   **提分点：** 在 Busybox 测试中，涉及大量小文件操作的脚本速度将显著提升。 [Redox OS AsyncScheme 设计](https://www.redox-os.org/news/io_uring-3/)

---

### **第四阶段：真机验证与竞赛结项展示（成果收割期）**

**目标：** 走出 QEMU，在真实的 VisionFive 2 硬件上锁定胜局。

*   **1. 真机硬件校验**
    *   在真实的 RISC-V 核心上运行测试。在真机上，`Acquire/Release` 的内存屏障成本和 MMU Walk 的开销会更真实，此时 `Fixed Buffers` 的优势将从 8% 扩大到 20% 以上。
*   **2. 产出三维对比报告**
    *   **① 响应时效：** 给出 `io_uring` 与同步调用的时延分布图（P50/P99）。
    *   **② 内存占用：** 展示在高并发 I/O 下，`io_uring` 节省的内核栈内存数据。
    *   **③ 通用性：** 展示同一套 `io_uring` 代码如何无缝驱动不同设备（Pipe, Net, Disk）。

### **针对提分操作的即时建议：**
如果你想立刻看到跑分提升，建议优先攻克 **`IORING_OP_SEND / RECV`**。网络性能在 StarryOS 这种基于协程的内核中非常容易出彩，因为你可以利用 `axnet` 的异步特性实现真正的“全链路异步”，这在 `Iperf` 测评中会有直接的加分。此外，针对 `Iozone` 的**异步预读**也是一个能快速产生亮眼数据的“黑科技”。