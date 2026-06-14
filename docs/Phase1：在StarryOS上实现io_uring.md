# Phase 1：在 StarryOS 上实现 io_uring（异步系统调用接口）

## Context（为什么做、做什么）

**竞赛实践任务**：在 QEMU/星光2 上选一个内核组件，用异步机制优化**①响应时效 ②内存占用 ③通用性（驱动跨 OS 复用）**。Embassy 非必须，复用现有 axtask 异步基建即可。串口方向已被 rival（`daivy2333/asyncuart-dev`）做透，故**改做 io_uring**——它是「异步系统调用」的标准答案，StarryOS 完全空白（`io_uring_setup` 现为 ENOSYS），不在导师清单内，与所有已列 prior work 零撞车，且三项评判全中。

**可行性已源码核实**（见 `docs/io-uring-feasibility.md`）：`SharedPages`+`Backend::new_shared` 提供 kernel/user 共享环形内存；`Pollable`/`poll_io` 可被任意任务复用；linux-raw-sys 已 vendored 全部 io_uring 定义（syscall 425/426/427、`io_uring_sqe/cqe/params`、offsets、opcodes、`IORING_REGISTER_BUFFERS`）。

**核心设计决策**（决定全局形态）：`sys_io_uring_enter` 跑在**提交进程的任务上下文**里——其 fd 作用域与页表都处于活跃态。于是在 enter 路径里把 `fd→Arc<dyn FileLike>` 和「用户缓冲区→物理页」全部解析好，**只把已解析的 Arc + 物理地址**交给 worker。worker 是无上下文的纯内核任务，用 `phys_to_virt` 访问数据、用 `block_on(poll_io(file, …))` 驱动 I/O，**永不需要跨地址空间切换、跨作用域查 fd、或逐次拷贝**。这同时把 Opus 顾问的「固定缓冲(#1)」「上下文切换抽象(#2)」「内存序(#3)」三点收编为轻量机制；顾问 #4（AsyncRead/AsyncWrite trait）作为通用性加分项放在 stretch。

**关键约束（已验证，决定设计）**：`FD_TABLE` 是 `scope_local!`（`kernel/src/file/mod.rs:188`），仅由 `TaskExt::on_enter` 为带 `Thread` 扩展的任务激活（`task/mod.rs:157`）。`axtask::spawn` 的 worker 无 `Thread` → 落到全局表 → **worker 侧 `get_file_like` 必失败**。故 fd 解析必须在 enter 路径完成。

---

## 目标产物

一个可端到端运行的 io_uring MVP：用户态 `io_uring_setup` 拿 fd → mmap 三段环 → 提交 NOP/READ/WRITE/POLL_ADD SQE → `io_uring_enter` → 收 CQE；并产出**延迟 + 内存 + 通用性**三份基准对比。

## 模块布局（新增 `kernel/src/file/io_uring/`）

```
io_uring/
├── mod.rs      # pub struct IoRing（持有 SQ/CQ 视图 + 三组 Arc<SharedPages> + worker_in 队列 + registered_bufs）；impl FileLike
├── ring.rs     # SubmitQueue / CompletionQueue：SharedPages 上的 u32 head/tail（Acquire/Release）+ io_sqring_offsets/io_cqring_offsets
├── op.rs       # struct Op{ file: Arc<dyn FileLike>, buf: PinnedUserBuf, user_data, opcode, len }；OpQueue（SPSC/Mutex<VecDeque>）
├── worker.rs   # worker_loop：pop Op → 按 opcode 分发 → post CQE；do_read/write/poll_add
├── buf.rs      # PinnedUserBuf（Vec<PhysAddr> + phys_to_virt 内核访问器）+ RegisteredBufTable（M3）
└── syscall.rs  # sys_io_uring_setup / enter / register
```

环内存复用：SQ ring / CQ ring / SQE 数组各一组 `Arc<SharedPages>`（`shared.rs:16` `SharedPages::new`），内核侧 `phys_to_virt` 读写、用户侧经 mmap 映射同一批物理页（`Backend::new_shared`，`shared.rs:107`）——与 System V shm 同路径。

## 三个系统调用 + mmap 路由

- **`io_uring_setup(entries, params)`**（425）：校验 entries（2 的幂，clamp）；分配三组 `SharedPages`；按 `io_sqring_offsets`/`io_cqring_offsets` 初始化 ring 控制字段并回填 `params.sq_off/cq_off`（Linux ABI 常量）；构造 `IoRing`；`axtask::spawn_with_name(worker_loop)`（`api.rs:144`）起 worker；`io_ring.add_to_fd_table(false).map(|fd| fd as isize)`——照搬 `sys_epoll_create1`（`io_mpx/epoll.rs:31`）。
- **mmap-the-fd**：在 `sys_mmap`（`mmap.rs` ~203 `FileBackend::Direct` 分支后）加一个 `downcast Arc<IoRing>` 分支，按 `offset ∈ {IORING_OFF_SQ_RING=0, CQ_RING=0x8000_0000, SQES=0x1_0000_0000}` 选中对应 `SharedPages`，返回 `Backend::new_shared(start, pages)`。约 10 行，集中且有注释。
- **`io_uring_enter(fd, to_submit, min_complete, flags, …)`**（426，**核心**）：跑在提交者上下文。
  1. `IoRing::from_fd(fd)`；
  2. 读用户 SQE（`UserPtr`/`access.rs`），`get_file_like(sqe.fd)`（提交者作用域 ✓），`PinnedUserBuf::resolve(sqe.addr, sqe.len)`（页表遍历，`PageTable::query` `bits64.rs:66`，校验权限、非 CoW-pending）；
  3. 组 `Op{ file, buf, … }` 入 `worker_in`；`sq.head` 用 `Release` 推进；
  4. 若 `IORING_ENTER_GETEVENTS`：`block_on(timeout(dur, 等 `cq.tail - cq.head ≥ min_complete`))`（`future/time.rs:145`）。
- **`io_uring_register`**（427）：M3 实现 `IORING_REGISTER_BUFFERS`（预 pin 一组 `PinnedUserBuf` 进 `RegisteredBufTable`，SQE 用 `READ_FIXED/WRITE_FIXED` 按索引引用）。

## Worker + 缓冲模型 + 内存序

- **worker_loop**：`pop Op`（`WaitQueue`/condvar 阻塞）→ `NOP`/`READ`/`WRITE`/`POLL_ADD` 分发 → `post_cqe`（`cq.tail` `Release`）。`shutdown` 时退出。
- **do_read/write**：复刻 `pipe.rs:123,156` 的 `block_on(poll_io(&*file, IoEvents::IN/OUT, false, || { file.read/write(buf) }))`。因 `file: Arc<dyn FileLike>` 已在 enter 解析、`Pollable::register` 无任务上下文依赖（`axpoll/src/lib.rs:58`），worker 可直接驱动任意 fd，**绕开作用域问题**。
- **do_poll_add**（M2）：`block_on(poll_fn)` 等 `file.poll()` 命中所请求 `IoEvents`——结构同 `epoll_wait` 单 fd 版，复用 `epoll.rs:176` 的 `file.poll()`。
- **内存序（RISC-V，必须对）**：SQ tail（用户→内核）用户 `Release`/内核 `Acquire`；SQ head 与 CQ tail（内核→用户）内核 `Release`/用户 `Acquire`；CQ head（用户→内核）用户 `Release`/内核 `Acquire`。负载写在 tail `Release` 之前（同核程序序天然满足）。`core::sync::atomic` Acquire/Release 足够，与 `axsync/mutex.rs` 既有用法一致。
- **缓冲**：M1 先做 inline（enter 时逐 op 页表遍历解析物理页，零拷贝、零跨空间）；M3 再做 registered（预 pin，省逐 op 页遍历，做 A/B 对比）。两者共用 `PinnedUserBuf` 抽象。

## 里程碑（每阶段都带可验证交付 + 风险解除）

- **M0 环骨架 + NOP 往返**：`io_uring_setup` 返 fd、mmap 三段、提交 NOP、收 CQE。用 `rdtime` 测环往返延迟，对比裸 `write(pipe)`。**解除风险**：SharedPages 环端到端、Acquire/Release 序对（NOP 不往返=序错）、mmap offset 路由。
- **M1 READ/WRITE on pipe + inline 缓冲**：提交 READ/WRITE SQE 到 pipe，worker 用 `poll_io` 跑通，字节落进用户缓冲。**解除风险**：`Arc<FileLike>` 交接绕开作用域、worker 侧 `phys_to_virt` 访问、`PageTable::query` 解析提交者页表得正确物理页、`block_on` 在 worker 任务里不 deadlock。
- **M2 POLL_ADD + 扩到 socket/file**：`IORING_OP_POLL_ADD` 在 pipe/socket 可读/可写时返 CQE。**解除风险**：任意 `Pollable` 统一驱动（通用性故事），证明每加一种 fd 类型零改动。
- **M3 基准 + registered buffers**：(a) inline vs `READ_FIXED` A/B；(b) 延迟（p50/p99，`rdtime`，10 万次）+ 内存（N 个并发在途 op：同步=N 线程栈，io_uring≈1 worker 栈+环页，用 `alloc_frame`/`SharedPages::new` 计数）对比；(c) 通用性（同一路径跑 pipe/socket/`/dev/zero`）。**命中三项评判**。
- **Stretch**：`AsyncRead/AsyncWrite` trait（把 `poll_io` 包成 `async fn`，io_uring worker 调 trait 而非 `FileLike::read`，供其他 OS/未来 Embassy 驱动复用——通用性加分）。

## 待改/待建文件（代表性）

**新建**：`kernel/src/file/io_uring/{mod,ring,op,worker,buf,syscall}.rs`；根目录或 rootfs 内的 C 微基准（`io_uring_nop.c`、`io_uring_pipe.c`，用裸 syscall 避 liburing 版本差）。
**修改**：
- `kernel/src/file/mod.rs` — `mod io_uring; pub use io_uring::IoRing;`
- `kernel/src/syscall/mod.rs` — 把 `Sysno::io_uring_setup`（623 行）移出 `sys_dummy_fd` 组，新增三条 dispatch 到 `sys_io_uring_setup/enter/register`；补 enter/register（426/427）。
- `kernel/src/syscall/mm/mmap.rs` — ~203 行后加 `downcast Arc<IoRing>` 分支按 offset 路由 `SharedPages`。

## 复用的现成函数（不要重造）

- `SharedPages::new` / `Backend::new_shared`（`shared.rs:16,107`）— 环内存。
- `get_file_like(fd)` / `FileLike::from_fd` / `add_to_fd_table`（`file/mod.rs:194,164,173`）— fd 解析与注册。
- `block_on` / `poll_io` / `timeout`（`axtask/src/future/{mod,poll,time}.rs`）— worker 异步驱动。
- `pipe.rs:123,156` 的 `block_on(poll_io(…))` 模式 — do_read/write 范本。
- `PageTable::query`（`page_table_multiarch/src/bits64.rs:66`）— 用户缓冲物理页解析。
- `sys_epoll_create1`（`io_mpx/epoll.rs:31`）—「建对象→返 fd」范本。

## 验证（docker + QEMU 端到端）

环境：`docker run -it --rm -v "$(pwd):/workspace" -w /workspace zhouzhouyi/os-contest:20260510 /bin/bash`；磁盘用 `sdcard-rv.img`（musl 布局，**勿用** `rootfs-riscv64.img`/`make/disk.img`，见 `docs/phase1-async-analysis.md`）；构建 `make build` → `ax-pci_riscv64-qemu-virt.bin`。

- 测试程序部署：`riscv64-linux-musl-gcc` 静态编译 C 微基准，loop-mount 写进 `sdcard-rv.img`，内核启动后运行。
- **M0 通过**：打印 `NOP cqe res=0 user_data=…`、退出 0；trace 见 worker 任务创建 + 一个 CQE；记录 `rdtime` 往返。
- **M1 通过**：io_uring WRITE 4KB 进 pipe、READ 回来、字节比对一致；打印逐 op 延迟。
- **M3 基准**：延迟表（sync / io_uring-inline / io_uring-fixed，p50/p99 ns）；内存曲线（pages vs 并发在途 op N=1..64，io_uring 近水平、同步线性）；通用性（pipe/socket/`/dev/zero` 同路径延迟）。

## 风险与解除

| 风险 | 严重度 | 解除点 |
|---|---|---|
| scope_local FD_TABLE → worker 解析 fd 失败 | **高（设计级）** | 设计绕开：enter 路径解析 `Arc<FileLike>`。M1 验证 |
| Acquire/Release 序错 → 空环/丢唤醒/死锁 | 高 | M0（NOP 必往返）+ M1（READ 必完成） |
| `PageTable::query` 对 CoW 页解析错物理页 | 中 | M1 字节比对；必要时 `populate_area`（`access.rs:57` 范式） |
| worker 内 `block_on` 死锁 | 中 | M1（pipe 已从任务上下文 block_on，worker 同理） |
| 用户中途 unmap 缓冲（TOCTOU） | 低（MVP） | MVP 接受，注明；M3 registered 缓解 |

> 顾问 #1（registered buffers）与 #4（async trait）正确，但**不应排在 READ/WRITE 之前**：registered 需要可对照的 inline 路径，async trait 是 M2 之后的通用性重构。本计划将其置于 M3/stretch。
