# io_uring 调度架构对比：Linux 完成驱动 vs StarryOS 轮询驱动

> 一份技术论证，回答一个问题：**StarryOS 的 io_uring 为什么是现在这个样子？要不要、以及怎样变得更像 Linux / tokio-uring 的完成驱动模型？**
> 配套文档：`docs/io-uring-final-report.md`（实现与数据）、`docs/StarryOS io_uring 性能登顶与架构演进宏观计划.md`（宏观路线）。

---

## 1. 起因

规模化对照（见 final-report §5.2）暴露了一个现象：96 并发连接时 `global_worker` 唤醒效率从 48 连接的 **10.3 CQEs/wake 崩到 0.0**——6124 次唤醒只收割 288 个 CQE，可靠性在 74/96 与 96/96 间波动。

最初的判断是"缺 SQPOLL"，后来被实测**证伪**（io_uring 在 ≥48 连接已确证胜过 epoll，与 SQPOLL 无关）。真正的瓶颈在 worker 的调度模型本身。本文把**两种调度模型**（Linux 完成驱动 / StarryOS 当前轮询）摊开来比，并给出可落地的综合方案。

> 写作中纠正了一个早先的误判：曾笼统说"StarryOS 驱动是纯轮询、不能 completion-driven"。代码核实后，**异步唤醒机制其实是存在的**（§5 详述）。本文以纠正后的理解为准。

---

## 2. TL;DR

| | 模型 A：Linux 完成驱动 | 模型 B：StarryOS 当前 worker |
|---|---|---|
| 唤醒粒度 | **per-op**（每个 socket 的 wait_queue 独立唤醒） | **广播**（任一事件 → worker 重扫全部 in-flight） |
| 唤醒语义 | "数据已就绪，来取" | "发生了点事，去重新查一遍" |
| 协议处理谁做 | **自主 softirq**（硬件中断驱动，独立于任何任务） | **惰性内联**：被唤醒者在 `socket.poll()` 里调 `iface.poll()` |
| 单事件工作量 | O(1) | O(inflight)，累计 **O(N²)** |
| 内存（连接级） | sk_buff / socket buffer 机械开销 | **O(1)**（1 worker 栈 + N 个小结构体） |
| 并发上限 | 极高（百万级，久经考验） | 实测可见天花板 ~96 连接 |
| 构建复杂度 | 巨大（softirq/NAPI/sk_buff/wait_queue 全套） | 小（~1500 行，单任务，协作调度） |
| 对 StarryOS 的契合度 | 需要重建中断驱动的网络栈 | 原生契合 Rust async 轮询驱动 |

**结论先行**：二者不是"谁对谁错"，是不同投入下的工程取舍。模型 A 是性能上限的参考，但它的核心前提（自主 softirq + 完整中断网络栈）StarryOS 没有、且不在 io_uring 子系统范围内。模型 B 简单且契合现状，但唤醒粒度太粗。**最优解是 B+：保留 B 的单 worker + O(1) 内存，只把"广播重扫"换成"per-fd 完成路由"，拿到 A 的 per-op 唤醒粒度。**（§7）

---

## 3. 术语：completion-driven vs poll-driven

- **completion-driven（完成驱动）**：提交一个操作 → 注册"完成回调" → 当前任务休眠 → **某个外部机制（中断/softirq/另一个线程）把操作真正做完** → 唤醒"只属于这个操作"的等待者。等待者醒来时，结果已就绪，它只负责"取"。代表：Linux io_uring + softirq、tokio-uring 的 CQ 消费循环。
- **poll-driven（轮询驱动）**：提交一个操作 → 把它包成 future 放进队列 → **一个循环任务反复 poll 所有 future**，直到某个 future 报告就绪。future 内部是"查一下就绪没，没就注册 waker 返 Pending"。代表：epoll（事件循环驱动）、本项目的 `global_worker`。

io_uring 的**对外**语义是 completion-driven（SQE 进、CQE 出）。本文讨论的是**内核内部**用什么模型把它实现出来——这是 96 连接抖动的根因所在。

---

## 4. 模型 A：Linux 完成驱动（参考实现）

### 4.1 数据流（一次 socket READ）

```
① 用户态提交 IORING_OP_READ（写 SQ 环）
② io_uring 内核：io_read() → sock_recvmsg，发现无数据
③ 在该 socket 的 sk_wq 注册 wait_queue entry，请求挂起
④ [硬件中断] NIC 收包 → softirq: net_rx_action()
⑤ softirq: ip_rcv → tcp_v4_rcv → 协议栈处理 → payload 进 sk_receive_queue
⑥ tcp 的 sk_data_ready() → 唤醒 wait_queue 上**该 socket** 的 entry
⑦ io_uring 重试：recvmsg 现在能读到了 → 填 CQE
⑧ 用户态在 CQ 上看到 CQE
```

关键点：**④⑤⑥ 由自主的 softirq 完成，独立于任何用户任务**；任务在 ⑥ 被唤醒时，数据**已经**在 socket buffer 里，⑦ 只是 dequeue。

### 4.2 优点

1. **per-op 唤醒、零重扫**：wait_queue 按 socket 维度，一个 socket 就绪只唤醒等它的那个 entry，不会波及其他 socket。单事件 O(1)。
2. **唤醒语义干净**：醒来即"数据就绪"，被唤醒路径不碰协议栈，只取数据。
3. **可扩展性强**：softirq + wait_queue 是为高并发设计的，单实例支撑百万连接。
4. **可叠加零 syscall 提交**：SQPOLL 内核线程后台扫 SQ 环，配合注册缓冲，实现"零 syscall I/O"。
5. **关注点分离**：协议处理（softirq，可跨核）与应用逻辑（进程上下文）解耦。

### 4.3 代价 / 缺点

1. **基础设施巨大**：NAPI、softirq 调度、sk_buff 生命周期、socket buffer 管理、wait_queue、跨上下文（硬中断→softirq→进程）锁层级——几十年的 Linux 演进。
2. **正确性复杂**：中断/softirq/进程三态之间的并发与锁极易出错。
3. **内存开销**：每个包走 sk_buff，每个 socket 一套 buffer 机制，per-packet 开销不小。
4. **从零构建不现实**：对一个 Rust 教学/比赛 OS 而言，重建一套中断驱动的网络栈，工作量远超 io_uring 子系统本身。

---

## 5. 模型 B：StarryOS 当前 worker（现状）

### 5.1 数据流（一次 socket READ，带代码佐证）

```
① 用户态写 SQE；io_uring_enter → submit_pending() 把 op 推进 GLOBAL_QUEUE
② global_worker（单任务，block_on + poll_fn）drain 队列 → OpFuture 入 inflight
③ worker 调 Pin::new(&mut f).poll(cx)：
     OpFuture::poll_rw → socket.poll_read(cx) → 没数据
     → file.register(cx, events)        [mod.rs:446]
     → 返 Poll::Pending，OpFuture 放回 inflight 队尾
   ⚠ 注册的是 worker 自己那一个 waker（cx 是 worker 的）
④ [事件] loopback send → poll.wake()   [loopback.rs:52]
         或 NIC 收包 → IRQ → register_irq_waker 唤醒  [ethernet.rs:338]
⑤ worker 被唤醒 → poll_fn 再跑一遍 → for 0..inflight.len() { poll each }
     [mod.rs:1248]  ← 把全部 in-flight 重 poll 一遍
     其中就绪的那个返回数据，其余返 Pending 又放回
⑥ progressed=true → poll_fn 返回 → 出循环 → 下一轮 poll_fn 又把剩下的全 poll
   ⚠ 每完成一个 op，就把剩余 N 个全扫一遍 → 总工作量 O(N²)
⑦ 完成的 op → post_cqe
```

> 注意 `socket.poll()`（tcp.rs:474）函数体第一行就是 `poll_interfaces()`——所以 worker 在 ⑤ poll OpFuture 时，顺带把 smoltcp 推进了一轮（包进 socket）。这是惰性协议处理（见 §6）。

### 5.2 优点

1. **简单、小**：~1500 行，单任务，无中断/softirq/跨上下文锁，协作调度，易调试易验证（10/11 上游测试通过）。
2. **O(1) 任务内存**：1 个 worker 栈 + N 个堆上的小 OpFuture 结构体。head-to-head 实测 K=64 时比 fork-per-connection 少 ~18MB（final-report §5.1）。
3. **原生契合 StarryOS 驱动模型**：smoltcp / pipe / fs 都是 Rust async 轮询式，worker 是这些驱动与完成环之间**天然且必要**的适配层。
4. **协作调度在单核上更可控**：SMP=1 下没有抢占式 IRQ 的时序复杂度。
5. **正确性扎实**：24/48 连接逐字节验证全过，96 连接多数轮全过。

### 5.3 代价 / 缺点

1. **O(N) 重扫 / O(N²) 总量**：`mod.rs:1248` 每次唤醒 poll 全部 in-flight；`progressed→重入→再全 poll` 使每完成一个 op 就全扫剩余。→ 96 连接 6124 唤醒 / 0.0 CQEs/wake。**这是当前可扩展性天花板。**
2. **广播式唤醒**：所有 OpFuture 共享 worker 的一个 waker（`mod.rs:446/468`）。一个 socket 的事件唤醒 worker，worker 却要触碰全部 socket——经典的 level-trigger epoll 病。
3. **内部本质是轮询**：对外 completion-driven，对内是 polling 包了一层。不是"真"完成驱动。
4. **完成不自带身份**：waker 不携带 op/fd 标识，无法把一次完成路由到具体 op。
5. **单全局 worker**：多个 io_uring 实例共用一个 GLOBAL_QUEUE + 一个 worker，未来可能互相饿死（final-report §8.2）。

---

## 6. 真正的差异：唤醒时机 & 协议处理归属

> 这一节是对"硬约束"误判的纠正。曾笼统断言"StarryOS 驱动纯轮询、不能 completion-driven"。核实代码后，**异步唤醒机制是有的**——差异比想象中窄。

### 6.1 唤醒机制其实存在（不是纯忙轮询）

- **loopback**：`send()` 入队后立即 `self.poll.wake()`（loopback.rs:52）——包一发送就 fire 已注册 waker。
- **物理 NIC**：`register_waker` → `register_irq_waker(irq, waker)`（ethernet.rs:338）——网卡硬中断直接唤醒注册的 waker。
- **smoltcp 定时器**（重传 / TIME_WAIT / 延迟 ACK）：`poll_at` → `sleep_until(next)`（service.rs:65/73）挂在 axtask timer wheel。

这套和 Linux 的 wait_queue 在"唤醒"这一层**同构**。tokio-uring 式"注册 waker、就绪被唤醒"完全可以做。

### 6.2 唯一的真差异：唤醒发生在流水线哪一段 + 协议处理归属

| | Linux | StarryOS (axnet-ng) |
|---|---|---|
| 唤醒时机 | softirq **处理完**包、payload **已进 socket 接收队列后**，才唤醒 | IRQ / loopback-send **一发生**就唤醒，此时包还在设备原始队列 |
| 协议处理（把包搬进 socket） | **自主 softirq**，独立于任何任务 | **没有自主 softirq**；由"查就绪的人"惰性在 `socket.poll()` 里调 `iface.poll()` 完成（tcp.rs:474） |
| 被唤醒者醒来后 | 数据已在 socket，`recv()` 直接取 | 必须自己先 `poll_interfaces()` 跑完协议栈，socket 才有数据 |

一句话：**Linux 的 wake = "数据已就绪，来取"；StarryOS 的 wake = "来处理一下再查"。**

### 6.3 这为什么不是 io_uring 的拦路虎

这个差异**不阻止** completion-driven 设计，只决定"被唤醒后要顺手做一次协议推进"。io_uring worker 每次 per-fd 唤醒后，`socket.poll()` 已经**内联** `poll_interfaces()`——所以是"每个就绪 fd 一次便宜的协议 pass"，不是额外开销。per-fd 完成路由（§7）天然吸收掉这个差异。

---

## 7. 多维对比表

| 维度 | 模型 A（Linux） | 模型 B（StarryOS 现状） | B+（建议） |
|---|---|---|---|
| 唤醒粒度 | per-op | 广播 | **per-fd** |
| 单事件成本 | O(1) | O(N) | **O(ready)≈O(1)** |
| 协议处理归属 | 自主 softirq | 惰性内联 | 惰性内联（可升级，见 §8） |
| 连接级内存 | 较高（sk_buff 机制） | O(1) | O(1)（保留） |
| 并发上限 | 极高 | ~96 可见天花板 | 显著抬升（推到更高） |
| 实现复杂度 | 巨大 | 小 | 中（~150–200 行） |
| 对 StarryOS 契合 | 需重建网络栈 | 原生契合 | 原生契合（B 的子集改造） |
| 调试难度 | 高（跨上下文） | 低（单任务） | 低（仍是单任务） |
| 是否需改 axnet-ng | — | 否 | 否（A-lite 为可选增强） |

---

## 8. 单核（比赛约束）下的诚实评估

比赛是 **SMP=1**。这把"模型 A 的优势"切掉了一半，需要诚实区分：

- **softirq 的"跨核并行"优势在 SMP=1 不存在**——softirq 也跑在同一个核上。
- SMP=1 下 A 相对 B 的**真实收益**只剩一条：**唤醒粒度**。A 在"数据就绪后"以 per-op 方式唤醒，被唤醒者不做协议活也不重扫；B 在"设备事件时"广播重扫。
- 而**唤醒粒度正是 B+（per-fd 路由）能拿到的**——不需要 softirq，不需要重建网络栈。所以在比赛的单核语境下，**B+ 几乎能吃下 A 在这里全部的实质优势**。
- 模型 A 真正不可替代的部分（绝对 syscall 消除的 SQPOLL、跨核协议 offload）在单核上要么无意义、要么需要真实多核硬件——属于"真实硬件部署"阶段，非当前瓶颈。

> 推论：单核语境下，追求"像 Linux"的边际收益递减极快；B+ 是投入产出比的甜点。

---

## 9. 综合建议

### 9.1 第 1 层（io_uring 范围内，现在可做）：B+ per-fd 完成路由

保留单 worker（O(1) 内存、协作调度、易调试），只换掉"广播重扫"：

- OpFuture 按 fd 分桶（`HashMap<fd, Vec<OpFuture>>` 或 slab）。
- 每个 OpFuture 用**带 fd 标签的 waker** poll（`Pollable::register` 本来就接受 `context.waker()`，worker 侧喂 per-fd waker 即可）。
- device wake（loopback/NIC IRQ）→ 标签 waker → 该 fd 入 `ready_set` + kick worker 一次。
- worker 醒来 → 只 drain `ready_set` → 只 poll 这些 fd 的 op（顺带内联 `poll_interfaces()`）→ 完成的 post CQE、出桶；仍 Pending 的重挂 per-fd waker。
- **旁路**：TIMEOUT 走独立 timer waker（本来就是）；NOP 立即完成。

效果：单事件从 O(inflight) 降到 O(ready)≈O(1)，总工作量 O(N²)→O(N)。预期把 96 连接的 0.0 CQEs/wake 拉回 ~1.0 量级、可靠性收敛。

**量级**：~150–200 行，集中在 `global_worker` 重写 + 一个 per-fd waker 类型。唯一 fiddly 点是 Rust 自定义 Waker（查 axtask 的 `AxWaker` 能否组合/携带 tag）。

### 9.2 第 2 层（axnet-ng 范围内，可选、更深）：A-lite IRQ 后台 poller

若要追求最纯粹——worker 醒来时数据已在 socket、它只 recv + post CQE、从不碰协议栈——在 axnet-ng 加：

> **NIC-IRQ 驱动的后台 poll 任务**：每次网卡 IRQ（或 loopback wake、或 timer）先调 `Service::poll()` 把包推进 socket，**然后**才 fire socket waker。

这是"自主 softirq"的 StarryOS 等价物。有了它，io_uring worker 退化成纯"注册 per-fd → socket 就绪唤醒 → recv → CQE"，精神上与 Linux io_uring / tokio-uring 一致。

**优先级**：低于第 1 层。第 1 层已能拿到 ~95% 收益；第 2 层是"更像 Linux"的使能项，等真实硬件/更高并发需求出现再做。

---

## 10. 结论

- **模型 A 不是"更对"，是"更重"**：它的 per-op 唤醒和干净语义，代价是一整套自主 softirq 基础设施，对 StarryOS 是重建网络栈级别的工作量，不在 io_uring 子系统范围内。
- **模型 B 不是"更错"，是"更粗"**：唤醒粒度太粗（广播重扫）是 96 连接抖动的唯一实质根因，但它保留了简单、O(1) 内存、契合现状的全部优点。
- **B+ 是二者的正确综合**：保留 B 的工程优点，只借用 A 的 per-op 唤醒粒度。在比赛的单核语境下，B+ 几乎吃下 A 在这里全部的实质优势，且无需重建任何基础设施。
- **不选什么**：不重建中断网络栈（A 全套）、不做 per-op-task（最像 tokio，但每 task 一个栈 = O(K) 内存，砸掉 O(1) 卖点）、不优先做 SQPOLL（单核上非瓶颈）。

一句话：**问题的本质不是"轮询 vs 完成"，而是"广播重扫 vs 精准路由"。B+ 用最小改动把后者补上。**
