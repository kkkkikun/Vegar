# 第一阶段分析报告：StarryOS 串口 / 异步现状（动态跟踪 + 源码核对）

> 分支：`ax-pci` ｜ 架构：`riscv64gc-unknown-none-elf`（QEMU `virt`，PCI virtio）｜ 构建产物：`workspace_riscv64-qemu-virt.{elf,bin}`（CI 构建，`log_level=trace`）
>
> 方法：docker 镜像 `zhouzhouyi/os-contest:20260510` 内跑 QEMU，串口输出落盘后 grep；关键路径逐函数读源码核对。**本阶段未改动任何内核代码。**

---

## 0. 结论速览（最重要的纠偏）

任务书假设的基线是「同步阻塞串口 + 基于 wait_queue 的线程阻塞」。实测与源码核对后，真实情况是：

| 路径 | 真实状态 | 对后续阶段的意义 |
|---|---|---|
| **串口 RX（输入）** | ✅ **已经中断驱动 + 异步化**。UART RX(IRQ 10) → `irq_hook` → `PollSet::wake` → `AxWaker` → 唤醒被 `poll_io` 挂起的 `tty-reader` 任务。实测 `IRQ: external 10` 触发 **163208 次**，并伴随 `task unblock: Task(4, "tty-reader")` + `context switch ... -> tty-reader`。 | Embassy 不是「从零引入异步」，而是**替换/并存**这套已有的 per-task 阻塞执行器。`register_irq_waker` 的 IRQ→Waker 模式可直接复用。 |
| **串口 TX（输出）** | ⚠️ **真正的忙等**。`send_raw` → `retry_until_ok!` → `core::hint::spin_loop()` 死等发送寄存器。 | TX 异步化（中断/DMA 驱动）是少数确有收益的改造点之一。 |
| **WaitQueue** | 基于 `event_listener::Event`，**不是 `core::task::Waker`**；`wait()` 内部走 `block_on(listener)`。公开 API 仍是「阻塞当前线程」。 | 与 Embassy/futures 体系不直接互通，是后续要架桥的位置。 |
| **异步执行器** | 已存在：`axtask::future::block_on`（per-task 阻塞执行器）+ `AxWaker`（wake=unblock_task）。**不是** Embassy 那种单线程协作式多任务执行器。 | 决定 Phase 2 的形态：引入 Embassy 执行器作为**第二套**调度实体，或改造现有执行器。 |

---

## 1. 环境与命令（实测可用）

容器（评测镜像，含工具链 / qemu 10.0.2 / gdb）：
```bash
docker run -it --rm -v "$(pwd):/workspace" -w /workspace zhouzhouyi/os-contest:20260510 /bin/bash
```

> **关键坑（已踩）**：根目录的 `rootfs-riscv64.img` / `make/disk.img` 是从 `github.com/Starry-OS/rootfs` 拉取的 **FHS 布局**（`/bin/busybox`），而 `src/main.rs` 的 `CMDLINE` 硬编码为 `/musl/busybox`（musl 布局）。用它启动会在 `kernel/src/entry.rs:27` panic：`Failed to resolve executable path: NotFound`。
> **正确磁盘**：比赛专用镜像 `sdcard-rv.img`（含 `/musl/busybox`、`lmbench`/`unixbench` 测试脚本）—— 已就位根目录。

构建 / 运行（仓库根 Makefile，默认 `ARCH=riscv64`、`BUS=pci`、`SMP=1`）：
```bash
make build          # 产出 ax-pci_riscv64-qemu-virt.{elf,bin}（注意：CI 产物名为 workspace_*）
make run            # defconfig → 构建 → QEMU 启动（-nographic）
make rv             # = make ARCH=riscv64 run
make debug          # QEMU 以 -s -S 启动并自动 gdb attach（:1234，断点 __axplat_main）
make LOG=trace run  # 开 trace（CI 产物本身即 trace 构建）
```

直接跑现有 CI 产物（跳过重建，用正确磁盘 + 串口落盘）：
```bash
qemu-system-riscv64 -m 1G -smp 1 -machine virt -bios default \
  -kernel workspace_riscv64-qemu-virt.bin \
  -device virtio-blk-pci,drive=disk0 \
  -drive id=disk0,if=none,format=raw,file=sdcard-rv.img \
  -device virtio-net-pci,netdev=net0 -netdev user,id=net0,hostfwd=tcp::5555-:5555 \
  -serial file:boot.log -monitor none -display none
```
> 本地 `~/.local/bin/qemu-system-riscv64` (9.2.1) **未编译 user 网络后端**，且 `sdcard-rv.img`/`disk.img` 为 root 所有无法被普通用户打开——**务必在 docker（root）内跑**。

实测启动结果（`sdcard-rv.img`，60s，`LOG=trace`）：**0 panic**，86768 行输出，OSComp 测试套件完整运行（`runtest.exe`/`run-static.sh`、syscall、信号、COW 映射、context switch 全量）。

---

## 2. TX 路径（输出 · 忙等 · 静态确认）

```
println!/axlog
  → axhal::console::write_bytes()                 vendor/axplat-riscv64-qemu-virt/src/console.rs:23
  → MmioSerialPort::send_raw()                    vendor/uart_16550/src/mmio.rs:102
  → retry_until_ok!(try_send_raw())               vendor/uart_16550/src/lib.rs:69
  → loop { if OUTPUT_EMPTY { 写 data 寄存器 } else { core::hint::spin_loop() } }   ← 死等
```
- `try_send_raw`（mmio.rs:108）查 `LineStsFlags::OUTPUT_EMPTY`，不空即 `Err(WouldBlockError)`；`send_raw` 用 `retry_until_ok!` 宏死循环重试。
- **动态佐证**：boot2/boot3.log 全部 8.6w+ 行控制台输出，每一字节都过这条忙等路径（trace 下可见大量 `write_bytes`/`send_raw` 无独立 trace，但输出本身即证据）。
- **改造点**：把 `retry_until_ok!` 换成「注册 TX-就绪 Waker + `poll_fn`」，由 TX-holding-register-empty 中断唤醒（16550 `int_en` 可配 THRE 中断）。

---

## 3. RX 路径（输入 · 中断驱动异步 · 静态链 + 动态实锤）

### 3.1 静态调用链（逐函数核对）
```
用户敲键 → UART RX FIFO
  → RISC-V S-mode 外部中断 (S_EXT)
  → axplat handle(): PLIC.claim() → irq=10          vendor/axplat-riscv64-qemu-virt/src/irq.rs:207-216
        trace!("IRQ: external 10")                            (irq.rs:213)
        IRQ_HANDLER_TABLE.handle(10); PLIC.complete()         (irq.rs:214-215)
        return Some(10)
  → axhal irq_handler(): handle()=Some(10)            vendor/axhal/src/irq.rs:36-45
        IRQ_HOOK(10)  // 全局唯一 hook 槽 (AtomicUsize)        (irq.rs:40-44)
  → irq_hook(10)                                      vendor/axtask/src/future/poll.rs:51
        POLL_IRQ.lock().get(&10) → PollSet                     (poll.rs:52)
  → PollSet::wake() → 丢掉旧 inner → Drop 逐个 Waker.wake()   vendor/axpoll/src/lib.rs:133,104-109
  → AxWaker::wake_by_ref() → unblock_task(task)       vendor/axtask/src/future/mod.rs:42-48
  → 任务从 block_on 恢复 → poll_io 重poll → Console::read → read_bytes → try_receive() 取字节
        kernel/src/pseudofs/dev/tty/ntty.rs:16 ; console.rs:38 ; mmio.rs:125
```
- **挂起点**：`tty-reader`（或任一读端）在 `poll_io` 里 `WouldBlock` → `Pollable::register(cx, IN)` → `register_irq_waker(10, &waker)`（poll.rs:43）→ `set_enable(10, true)`（poll.rs:60）使能 PLIC IRQ 10，然后 `block_on` 把任务 `blocked_resched`。
- **关键纠偏**：IRQ 上下文**没有独立环形缓冲**；字节留在 UART FIFO，直到被唤醒的读端 `try_receive` 取走。任务书「IRQ 存入 ring buffer」的描述不成立。
- **TTY 接线**：`new_n_tty()`（ntty.rs:31-44）在 `irq` 特性下用 `ProcessMode::External(Box::new(|w| register_irq_waker(irq, &w)))`，`irq_num()` 返回 `Some(0x0a)`（console.rs:51）。`irq` 特性已开（`make/features.mk`: `lib_features := ... irq ...`）。

### 3.2 动态实锤（boot3.log，注入串口输入后）
```
[1.457161 0:3]  context switch: Task(3,"gc") -> Task(4,"tty-reader")
[1.458769 0:4]  task block: Task(4,"tty-reader")          ← tty-reader 武装 IRQ10 并阻塞(blocked_resched)
[1.459139 0:4]  context switch: Task(4,"tty-reader") -> Task(2,"main")
...（约 8.4s 后注入串口输入）...
[9.903204 0:73 irq:213]            IRQ: external 10        ← UART RX 中断，handle() 命中 trace
[9.903823 0:73 run_queue:261]      task unblock: Task(4,"tty-reader")   ← irq_hook→PollSet→AxWaker→unblock
[9.904226 ...  ]                   IRQ: external 10 (连续多字节)
[9.936628 0:73 run_queue:518]      context switch: Task(73,"busybox") -> Task(4,"tty-reader")  ← 调度恢复读端
```
统计：`IRQ: external 10` = **163208** 次（`IRQ: timer` = 4668 次）。RX 中断远多于预期——**疑点/后续优化点**：可能 FIFO 未及时排空导致中断风暴，Phase 3/4 可核查。

---

## 4. WaitQueue / 阻塞-唤醒机制（静态）

`vendor/axtask/src/wait_queue.rs`：
```rust
pub struct WaitQueue { event: event_listener::Event }     // :31
pub fn wait(&self) { listener!(self.event => l); block_on(l) }   // :51-54  ← 复用 block_on
pub fn notify_one(&self, resched: bool) { self.notify_many(1, resched) }   // :117
pub fn notify_many(&self, n, resched) { let k=self.event.notify(n); if resched {yield_now()} k } // :124
```
- `wait()` 把当前任务 `Running→Blocked` 并 `resched()`（经 `block_on`→`blocked_resched`，`future/mod.rs:75`）；`notify_*` 经 `event_listener::Event::notify` 唤醒。
- **核心结论**：WaitQueue 内部虽借 `event_listener` 的 `Listener: Future` 跑在 `block_on` 上，但其唤醒机制是 `event_listener::Event`，**不是 `core::task::Waker`**。与 Embassy/futures 的 Waker 体系不直接互通——这是 Phase 2+ 要架的桥。

---

## 5. 现有异步执行器（block_on / AxWaker）——决定 Phase 2 形态

`vendor/axtask/src/future/mod.rs`：
- `AxWaker{task: WeakAxTaskRef, woke: SpinNoIrq<bool>}`（:23）；`impl Wake`（:37）；`wake_by_ref` → `select_run_queue().unblock_task(task,false)`（:42-48）。**唤醒 future = 把对应 task 重新放回就绪队列**。
- `block_on<F: IntoFuture>`（:55）：为当前 task 建一个 `AxWaker`，循环 `fut.poll(cx)`；`Pending` 则 `blocked_resched`（挂起当前 task，:75），被 wake 后再 poll。

**形态判断**：这是 **per-task、阻塞当前线程** 的执行器——「一个 task 阻塞在一个 future 上，调度器切换到别的 task」。它**不是** Embassy 那种「单线程协作式轮询一堆任务 + 优先级抢占」。两者本质不同：
- 现状：协作式仅在单个 future 内部（await 点让出），跨 task 是**抢占式**（axtask 调度器）。
- embassy_preempt 目标：在专用线程上协作轮询多任务 + 按优先级抢占。
- ⇒ Phase 2 引入 Embassy 多半是**新增第二套执行器**（专用内核线程跑 `embassy_executor::run`），而非替换 `block_on`；`AxWaker`/`register_irq_waker` 这套 IRQ→Waker 可直接给 Embassy 的 `Spawner`/`Waker` 复用。

---

## 6. IRQ 分发机制（静态 · 单一 hook 槽）

`vendor/axhal/src/irq.rs`：
```rust
static IRQ_HOOK: AtomicUsize = AtomicUsize::new(0);          // :12  全局唯一
pub fn register_irq_hook(hook: fn(usize)) -> bool { /* 只能注册一次，CAS 0→hook */ }  // :19
#[register_trap_handler(IRQ)] pub fn irq_handler(vector) -> bool {     // :35
    if let Some(irq) = handle(vector) {                                // :39  PLIC claim
        let hook = IRQ_HOOK.load(..); if hook!=0 { hook(irq); }        // :40-44  对每个 IRQ 都调
    }
}
```
- `handle()`（`axplat-riscv64-qemu-virt/src/irq.rs:185` 的 `match`）：`@S_TIMER`→timer handler；`@S_SOFT`→IPI；`@S_EXT`→PLIC.claim→`IRQ_HANDLER_TABLE.handle(irq)`→complete→`Some(irq)`。
- **两条分发路径并存**：① `IRQ_HANDLER_TABLE`（设备自注册的 per-IRQ handler，virtio 等用）② 全局唯一 `IRQ_HOOK`（异步 poll 用，= `irq_hook`，按 `POLL_IRQ` BTreeMap 二次分发）。UART(10) 在 `IRQ_HANDLER_TABLE` 无注册项，仅靠 `IRQ_HOOK`→`irq_hook` 唤醒 poll。
- **对本项目**：PCI virtio（本分支）未见 `IRQ: external`（163208 全是 UART 的 10）——virtio 走 MSI/MSI-X 或轮询，不经 PLIC S_EXT。Embassy 若要接 virtio 完成中断，需另查 PCI/MSI 路径；接 UART/定时器则现成可用。

---

## 7. 现有异步基建清单（Embassy 落地的地基，非空白起点）

| 组件 | 位置 | 作用 |
|---|---|---|
| `block_on` | `vendor/axtask/src/future/mod.rs:55` | per-task 阻塞执行器 |
| `AxWaker` | 同上 :23-49 | `impl Wake`，wake=unblock_task |
| `poll_io` | `vendor/axtask/src/future/poll.rs:17` | 同步非阻塞 IO → async（WouldBlock 时注册 Waker） |
| `register_irq_waker` | 同上 :43 | IRQ→PollSet→Waker（**Embassy 可直接复用模式**） |
| `irq_hook` | 同上 :51 | 经 axhal 单一 hook 槽接收所有 IRQ |
| `interruptible` | mod.rs:106 | future 可被信号中断（`poll_interrupt`） |
| `PollSet`/`Pollable`/`IoEvents` | `vendor/axpoll/src/lib.rs` | 多 Waker 注册 + 事件位 |
| `WaitQueue` | `vendor/axtask/src/wait_queue.rs` | `event_listener` 驱动 |
| Pipe | `kernel/src/file/pipe.rs` | 已用 `poll_io`+axpoll 异步化 |
| 定时器 | `vendor/axtask/src/future/time.rs` + `axplat.../time.rs` | Embassy `embassy-time` 需对接的现成计时源 |

---

## 8. 后续阶段切入点（Phase 2-4）

1. **TX 异步化（Phase 3，收益明确）**：`send_raw` 的 `retry_until_ok!` 死等 → 改 THRE（发送保持寄存器空）中断 + `poll_fn`；复用 `register_irq_waker` 模式注册 TX-就绪 Waker。
2. **WaitQueue → futures 桥（Phase 2/3）**：WaitQueue 用 `event_listener::Event`，需包一层把其唤醒转成 `core::task::Waker`，或用 `poll_fn`+`register` 重建。
3. **Embassy 执行器引入（Phase 2）**：以专用内核线程跑 `embassy_executor`；`embassy-time` 对接 `axplat` 的 `mtime`/定时器（`axplat-riscv64-qemu-virt/src/time.rs`，`irq_num()` 在 time.rs:60）。难点：与 axtask「无栈协程 + 有栈线程」混合调度。
4. **IRQ→Waker 复用**：UART/定时器直接套 `register_irq_waker`；virtio(PCI/MSI) 需另查中断路径（本分支未见 PLIC 外部中断）。
5. **RX 中断风暴（疑点）**：`IRQ: external 10` 163k 次远超注入量——核查 FIFO 排空/中断清除是否高效，可能是 Phase 4 响应时延优化的意外大头。
6. **性能基线（Phase 4）**：磁盘内自带 `lmbench_testcode.sh`/`unixbench_testcode.sh`（`/musl/`），可直接做改造前后 context-switch / 响应时延对比。

---

## 9. 验证证据附录

- 启动日志（错磁盘，演示 panic）：`boot.log`（`entry.rs:27 Failed to resolve executable path`）。
- 启动日志（正确磁盘 `sdcard-rv.img`，trace）：`boot2.log`（0 panic，OSComp 全跑）。
- RX 注入实验（`-serial pipe:` 注入 `a` ×4）：`boot3.log`（`IRQ: external 10` ×163208，含 9.903s 中断→unblock→ctxsw 序列）。
- 复现命令见 §1；gdb 交互式断点跟踪脚本见 `scripts/trace-serial-gdb.md`（供后续阶段 backtrace 用）。

### 第一阶段完成判据
- [x] docker 内能启动到 OSComp 测试运行（0 panic）。
- [x] trace 日志可读，截取 RX 片段佐证（§3.2）。
- [x] 实测 `IRQ: external 10` 命中并伴随 `task unblock`+`context switch -> tty-reader`（坐实 RX 中断驱动异步链）。
- [x] TX 链、RX 链、WaitQueue 机制、IRQ 分发、异步执行器现状全部源码核对完成。
- [x] 本文档完成（3 链 + 基建清单 + 后续切入点）。
