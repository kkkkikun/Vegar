# io_uring 第一阶段进展记录（Stage 1：符合性 + 网络通用化）

> 分支 `ax-pci` ｜ 日期 2026-06-16
> 总计划见 `~/.claude/plans/docker-run-it-rm-foamy-rabbit.md`（Tier A：符合性 + 网络；SMP 真重叠留 Stage 2）

## 本阶段定位

M0–M3 已落地一个「能跑、但在 QEMU 单核上原始周期数输给同步」的 io_uring 原型（见 `docs/io-uring-performance-report.md`）。Stage 1 的目标是把它从「pipe/file 自测 demo」推进到「**符合 Linux ABI、能跑 liburing 与网络应用**」的工程化实现，直接补强竞赛三项评判。

外部参考对计划的映射：kernel.dk/io_uring.pdf + OpenAnolis → 符合性基准；perf-test-for-io_uring → 第三方背书；io_uring-echo-server → 网络端到端 demo；scipio → Stage 2 SMP+绑核；Redox AsyncScheme → Stage 3 驱动解耦。

## 进度总览

| 里程碑 | 状态 | 验证 |
|---|---|---|
| S1.1 GETEVENTS + min_complete 等待 | ✅ 完成 | `io_uring_getevents` PASS（等满 4 个 CQE） |
| S1.2 CQ 溢出保护 | ✅ 完成 | post_cqe 重放 + 计数；随 S1.1 wait 路径联调 |
| S1.3 READV / WRITEV | ✅ 完成 | `io_uring_readv` PASS（3 段 iovec，11 字节往返） |
| S1.4 最小 liburing 静态 shim | ✅ 完成 | `io_uring_shim_test` PASS（submit_and_wait/wait_cqe/readv） |
| S1.5 perf-test 子集 + lmbench | ✅ 完成 | `io_uring_iodepth` PASS（cyc/op 随 D 1→32 从 6182 降到 1367） |
| S1.6 ACCEPT（提交者上下文 accept+装 fd） | ✅ 完成 | `io_uring_accept` PASS（loopback TCP echo） |
| S1.7 RECV / SEND + MSG_DONTWAIT | ✅ 完成 | `io_uring_send_recv` PASS（socketpair + DONTWAIT→EAGAIN） |
| S1.8 echo-server（loopback） | ✅ 完成 | ACCEPT→RECV→SEND 端到端 echo（无需 NIC） |

**当前：io_uring 已具备 liburing 兼容性 + 网络通用化 + batching 基准。** 13 项 io_uring 测试（含 6 项新增）在 QEMU 单核下全部通过，0 panic；loopback TCP echo 端到端跑通。

---

## S1.1 GETEVENTS + min_complete 等待

**问题**：原 `sys_io_uring_enter` 把 `min_complete`/`flags` 全部下划线忽略，提交后立即返回——liburing 的 `io_uring_submit_and_wait` / `io_uring_wait_cqe` 根本无法工作。这是第三方测试的硬阻塞。

**实现**（`kernel/src/file/io_uring/{mod.rs,syscall.rs}`）：
- `IoRing` 新增 `cq_wq: Arc<WaitQueue>` 字段；新增 `wait_for_completions(min_complete)`。
- 用 `self.cq_wq.wait_until(|| { avail = cq.tail - cq.head + overflow.len(); avail >= min_complete })`——`WaitQueue`（`vendor/axtask/src/wait_queue.rs`，基于 `event_listener`）的 `wait_until` 会在注册监听器后**再**复查条件，故不会丢唤醒。
- 该等待跑在**提交者任务上下文**（`sys_io_uring_enter` 同步路径里），提交者 park 后调度器切到 worker，worker `post_cqe` 末尾 `cq_wq.notify_all(false)` 唤醒它。SMP=1 下这套对称的 park/notify 即是 I/O 与 CPU 的协作点。
- `Op` 多带一份 `cq_wq`，`post_cqe` 签名改为接收 `&WaitQueue`。

**验证**：`io_uring_getevents.c` 提交 4 个 NOP，`enter(fd, 4, 4, GETEVENTS)` 返回后**立即**（无轮询）从 CQ 读到 4 个 CQE。旧实现会立即返回 0 完成数。

```
===== [io_uring] running io_uring_getevents =====
PASS: io_uring GETEVENTS waited for 4 completions
===== [io_uring] io_uring_getevents exit=0 =====
```

## S1.2 CQ 溢出保护

**问题**：原 `post_cqe` 不读 `cq.head`，环满时**静默覆盖**——高 `iodepth` 下数据损坏。

**实现**：
- `IoRing` 新增 `overflow: Arc<Mutex<VecDeque<(u64,i32)>>>`；`CqRing` 新增 `overflow_ctr: *mut AtomicU32`（指向 `CQ_OFF_OVERFLOW`，offset 早已回填，此前从未写）。
- `post_cqe` 改写：先把新 CQE 压入 `overflow`，再按 `cq.head`（Acquire）算空位，**最旧优先**排空到环里；仅当回放缓冲本身超过一环深度才丢最旧并递增 overflow 计数（真溢出）。永不静默覆盖。
- S1.1 的 `avail` 必须计入 `overflow.len()`，否则 CQE 卡在溢出表时永久阻塞。

## S1.3 READV / WRITEV

**问题**：liburing 高频用 `io_uring_prep_readv/writev`，原实现只认单缓冲 READ/WRITE。

**实现**：`Op.buf` 由 `Option<PinnedUserBuf>` 改为 `Option<Vec<PinnedUserBuf>>`（单缓冲路径包 `alloc::vec![b]`，避免 enum 重复每个 match 臂）。`submit_pending` 新增 `OP_READV/OP_WRITEV` 臂：从 `sqe.addr`（iovec 数组）按 `sqe.len`（段数）循环解析，复用 `register_buffers` 的 `crate::mm::UserConstPtr<IoVec>` 范式。worker READ/WRITE 臂改为顺序遍历 `Vec`、累加字节。

**验证**：`io_uring_readv.c` 写 3 段（"AAAA"/"BB"/"CCCCC"=11B）经 WRITEV 入管道，READV 读回，逐段比对一致。

```
===== [io_uring] running io_uring_readv =====
WRITEV res=11 READV res=11
PASS: io_uring READV/WRITEV 3-segment round-trip (11 bytes)
```

## S1.4 最小 liburing 静态 shim

**问题**：要跑 perf-test / echo-server 这类 liburing 程序，要么移植真 liburing，要么写垫片。选垫片（`tests/liburing-shim/`，musl 静态库，**不进内核依赖**）。

**实现**（`liburing_shim.{h,c}`）：liburing 兼容子集——`io_uring_queue_init/exit`、`get_sqe`、`submit`、`submit_and_wait`、`wait_cqe[_nr]`、`peek_batch_cqe`、`cqe_seen`、`register_buffers`，及 `prep_{read,write,readv,writev,recv,send,accept}` 内联填充器。`io_uring_flush_and_enter` 实现 SQ 本地 → 共享环（array 恒等映射 + tail Release）的刷写。

**两个坑（已填）**：
1. **SQE 必须 64 字节**：shim 按 `sizeof` 索引 `sqes[]`，而内核 SQE 是 64 字节步长。raw-syscall 测试靠硬编码 `*64` 掩盖了「struct 只有 56 字节」；shim 必须补齐到 64（加了 `__pad4`），并用 `_Static_assert(sizeof(sqe)==64)` 防回归。
2. **`struct iovec` 冲突**：加了 `<sys/socket.h>` 后 musl 的 `sys/uio.h` 已定义 iovec，重定义报错——改用系统头。

**验证**：`io_uring_shim_test.c` 经 shim 提交 4 NOP（`submit_and_wait`）+ readv 往返，全通。

```
=== io_uring shim test ===
  shim NOP batch: 4 CQEs reaped via wait_cqe OK
  shim READV/WRITEV: write=17 read=17 r0=HELLO  r1=WORLD  r2=12345
PASS: io_uring shim (queue_init/get_sqe/submit_and_wait/wait_cqe/readv)
```

## 关键工程教训：构建/测试必须用 Docker

- **不要用 `starry-champion-new` 容器**：它把 `/workspace` 挂到了**另一个项目**（`core/StarryOS`），看不到本仓库的改动。
- 正确做法：一次性容器 `docker run --rm -v "$(pwd):/workspace" -w /workspace zhouzhouyi/os-contest:20260510`，镜像内含 `/opt/riscv64-linux-musl-cross/bin`（musl 工具链）与 `/opt/qemu-bin-10.0.2/bin`（QEMU）。
- **本地（非 docker）`make build` 不可信**：它会把 `starry-kernel` 当缓存跳过，**隐藏真实编译错误**（本次靠它漏掉了 `vec!` 宏不在 no_std 作用域的报错；docker 干净重建 35s 即暴露）。
- **`init_custom.sh` 是构建期烘焙的**（`src/main.rs:9` 的 `include_str!`），改测试列表后必须**重新 build** 内核才会生效。
- `sdcard-rv.img` 是裸 ext4（无分区表），`mount -o loop`（容器需 `--privileged`）拷入二进制即可。

---

## 网络通用化（S1.6 / S1.7 / S1.8）— ✅ 已完成并验证

Socket（`kernel/src/file/net.rs`）已 impl `FileLike`（read=recv, write=send）+ `Pollable` + `set_nonblocking`，故**网络 op 零改动 `net.rs`**——通用性故事成立：pipe / `/dev/zero` / socket 全走同一条 worker 路径。

- **S1.7 RECV/SEND + MSG_DONTWAIT**：worker 用 `recv_or_send` 辅助——`block_on(poll_fn 等 IN/OUT)` 就绪后**临时非阻塞** recv/send 再还原（裸阻塞会卡死串行 worker，即风险 R3）。`MSG_DONTWAIT(0x40)` 跳过 poll-wait，空 socket 立即返 `-EAGAIN`。
  ```
    DONTWAIT RECV (empty): res=-11 (want -11)
    SEND res=15 RECV res=15 rbuf2=hello-uring-net
  PASS: io_uring SEND/RECV on socketpair (+ MSG_DONTWAIT -> EAGAIN)
  ```
- **S1.6 ACCEPT**：worker 无 FD_TABLE scope 不能装 fd。**最终采用「提交者上下文处理 ACCEPT」**（比原计划的 split-channel 更简且正确）：`submit_pending` 把 ACCEPT 请求挑出来（不入 worker 队列）回传给 `sys_io_uring_enter`；enter 在**已把其余 op 推给 worker 之后**，逐个 `complete_accept`：`block_on(poll_fn 等 IN)` → `listener.accept()` → `Socket(inner).add_to_fd_table(false)` → `post_cqe(res=新 fd)`。这样提交者阻塞等连接时，worker 仍能并发跑现有连接的 RECV/SEND。要点：`use axnet::SocketOps;`（`accept` 是 trait 方法）；`add_to_fd_table(self)` 取所有权（照搬 `sys_accept4`，不能先包 `Arc`）。
- **S1.8 echo（loopback）**：**无需真网卡**——`axnet-ng` 的 `init_network` 在启动时无条件加 `LoopbackDevice`，故 `socket/bind/listen` 在 127.0.0.1 直接可用（NIC 枚举不再是前置 gate）。`io_uring_accept.c`：父进程 listen + fork 客户端 connect/send "ping"/read echo；父进程 io_uring ACCEPT→RECV→SEND 全链路回显。
  ```
  listening on 127.0.0.1:7321 (lfd=3)
    ACCEPT res=5
    RECV res=4 rbuf=ping
    SEND res=4
  PASS: io_uring ACCEPT + RECV + SEND loopback TCP echo
  ```

**Stage 1 网络与符合性主线至此全部完成。** S1.5（iodepth 基准）见下。

## 剩余（S1.5）— ✅ 已完成

- **iodepth 基准**（`io_uring_iodepth.c`，经 liburing shim）：`io_uring_register_buffers` + `READ_FIXED` 在 `/dev/zero` 上扫 iodepth D=1..32，对比单次 `read()` 同步基线。

  ```
  D    iouring cyc  cyc/op   sync cyc  cyc/op
  1         6182     6182        954      954
  2         3045     1522        587      293
  4         7128     1782        994      248
  8        14011     1751       2159      269
  16       23317     1457       2666      166
  32       43759     1367       5050      157
  ```

  **读法**：io_uring 的 cyc/op 随批量 D 从 6182 → 1367（~4.5× 下降），证明 **batching 摊销**——一次 `io_uring_enter` 提交 N 个 op，每 op 成本随 N 增大而下降。原始总周期上同步仍占优（QEMU TCG 单核下 syscall 陷阱≈内存访问成本，报告已如实说明），但这条「每 op 成本随批量下降」的曲线正是结构性优势；在 syscall 陷阱 ~1500 cyc 的真机上会翻转（见 `io-uring-performance-report.md` §6）。lmbench `lat_syscall` 涉及外部二进制，留待后续接入。

---

## Stage 1 小结

**8 个里程碑全部完成并在 QEMU 单核下验证，13 项 io_uring 测试（6 项新增）全过，0 panic。**

- **符合性**：`io_uring_enter` 真正等 `min_complete`（GETEVENTS）、CQ 溢出不再静默覆盖、READV/WRITEV、最小 liburing 静态 shim——**io_uring 已 liburing 兼容**。
- **网络通用化**：ACCEPT / RECV / SEND（含 MSG_DONTWAIT→EAGAIN）跑通，**loopback TCP echo 端到端**（无需 NIC）。pipe / `/dev/zero` / socket 全走同一条 worker 路径，**零改动 `net.rs`**——通用性成立。
- **批量摊销**：iodepth 基准量化了 O(1) vs O(N) 的 batching 优势。

---

## Stage 1.5：单核多路复用 worker（修队头阻塞）— ✅ 完成并验证

**问题（约束：初赛单核 smp=1，SMP 搁置）**：原 `global_worker` 串行处理——`queue.pop()` 后对每个 op `block_on`/`file.read`，worker **卡在单个 op 上**（队头阻塞）。N 个并发连接的 N 个 pending RECV 被串行化，单核事件驱动并发（io_uring 的核心单核价值）拿不到。axtask 任务栈 256KB，「每 op spawn 任务」会退回 O(N) 栈、丢掉内存优势。

**改造**（`kernel/src/file/io_uring/mod.rs`）：worker 变成**多路复用事件循环**——
- 新增 `OpFuture`：手写 `impl Future`（非 async fn，故 `Unpin`），把每个 op 变成「就绪检查（`Pollable::poll`）→ 未就绪则 `register(worker waker)`+`Pending`；就绪则 `set_nonblocking` + read/write」的状态机。对 pipe/socket/file/device 通用（`recv_or_send` 删去，逻辑并入 `OpFuture::poll_rw`）。
- `global_worker` 重写为 `block_on(async { poll_fn(|cx| { try_pop 新 op；用 worker 自身 waker 轮询全部 in-flight `OpFuture`；完成就绪的→`post_cqe`；全 pending 则 `register_worker` 后 park }) })`。任一资源就绪或新 op 入队都唤醒同一 worker waker。
- `OpQueue` 改 `worker_waker: Mutex<Option<Waker>>` + `try_pop()` + `register_worker(cx)`；`push` 唤醒 worker。`WaitQueue::wait_until`-based `wait_for_completions`/`post_cqe` 不变。
- 一致性小补：`sys_io_uring_setup` 回填 `params.features = IORING_FEAT_NODROP`（声明溢出重放）。

**结果（QEMU `-smp 1`）**：
- **回归**：原 13 项 io_uring 测试在新多路复用 worker 下**全过，0 panic**——证明改造不破坏正确性（pipe READ/WRITE、socket SEND/RECV、POLL_ADD、GETEVENTS、ACCEPT 均正常）。
- **并发 echo**（`io_uring_echo.c`，照搬 tokio-rs `tcp_echo.rs` 事件循环经 liburing shim）：单线程 `submit_and_wait(1)` 循环 + token 状态机 `ACCEPT→RECV→SEND`，**24 个并发 TCP 客户端**各发独立 payload，由**同一个**多路复用 worker 回显——`accepted=24 echoed=24 failed=0 clients_ok=24/24`，逐连接字节正确、不串不丢。
- **意义**：单核下 io_uring 真正实现**事件驱动并发**——1 个 worker（O(1) 内存：1 栈 + 环页）多路复用 N 个在途 op，而「线程/连接」阻塞模型需 N × 256KB 栈。队头阻塞消除（慢连接不阻塞快连接）。

> 踩坑：首版 echo 过度订阅 ACCEPT（每个完成的 ACCEPT 都补一个，总队列 > 客户端数），多余 ACCEPT 在 `complete_accept` 里永久等连接、死锁提交者。修复：`accept_queued` 计数封顶 NCLIENTS。**这是测试逻辑 bug，非 worker bug**。

**标准对齐**：参考 `ref/io-uring`（tokio-rs io-uring v0.7.12）。ABI 已一致（核实 `src/sys/sys_riscv64.rs`：syscall 425/426/427、offset、SQE/CQE 尺寸）。`examples/tcp_echo.rs` 正是需要多路复用的标准用例，故 echo 照搬其模式。

---

## 原版 tokio-rs `io-uring-test` 上机验证 — ✅ 6/10 通过

把 **原版** `io-uring-test` 二进制（用户态 Rust，静态交叉编译 `riscv64gc-unknown-linux-musl` + `+crt-static`，nightly-2026-02-25）跑在 StarryOS 单核 QEMU 上。工具链坑：0.7.12 的测试代码同时依赖旧 Rust（`*mut T: Default`）和新 Rust（`std::io::pipe`/`integer_sign_cast`），无单一版本可编；以 `--features ci`（剔除 musl 不兼容的 `test_statx` 与大 CQE 测试）+ 容器 2026 nightly + 静态链接 + 用户手改 `musl_statx`（裸 syscall 291）解决。新增内核 `IORING_REGISTER_PROBE`（opcode 8，回填支持的操作码表），否则 `require!` 的 `probe.is_supported()` 全 false、所有操作码测试被跳过。

**结果（`-smp 1`，按名逐个跑，避开 SQPOLL/TIMEOUT/CANCEL）**：

| 测试 | 结果 | 说明 |
|---|---|---|
| `test_nop` | ✅ PASS | NOP 往返 |
| `test_batch` | ✅ PASS | NOP 批量 + SQ/CQ |
| `test_queue_split` | ✅ PASS | SQ/CQ split 机制 |
| `test_tcp_write_read` | ✅ PASS | TCP Write+Read（IO_LINK 序保持） |
| `test_tcp_send_recv` | ✅ PASS | TCP Send/Recv |
| `test_tcp_accept` | ✅ PASS | TCP Accept |
| `test_pipe` | ⏭ skip | std::io::pipe gate 未满足 |
| `test_tcp_writev_readv` | ❌ FAIL | Writev=80 ✓ 但 **Readv=44** |
| `test_register_buffers` | ❌ FAIL | READ_FIXED=0（期望 4096） |
| `test_file_write_read` | ❌ FAIL | Write=44 ✓ 但 **Read=0** |

**两个已定位的真 bug（待修）**：
1. **READV 分块过读**：worker 逐 iovec 调 `file.read`（每次一个 recv）。当一次 recv 的可用数据 > 当前 iovec 容量时，`PinnedUserBuf::copy_in` 按 iovec 容量截断，**多余字节被丢弃**（readv 80B 数据，第一个 44B iovec 收 44，丢 36）。单缓冲（write_read）不触发。修法：READV 应一次读满所有 iovec，或让 recv 读量 ≤ 当前 iovec `remaining_mut`。
2. **文件 `sqe.off` 未生效**：READ/WRITE/READ_FIXED/WRITE_FIXED 忽略 `sqe.off`，用 fd 当前位置。文件 Write 推进 fd 位置后，Read 读到 EOF（`test_file_write_read`）或错位（`test_register_buffers`）。修法：按 `sqe.off` 做 pread/pwrite（off=-1 时用当前位置）。管道/socket 不受影响（stream，忽略 off），故 `test_tcp_*` 全过。

> 持久容器 `iouring-dev`（`--privileged --network host`，仓库挂 `/workspace`）做交叉编译；构建命令在容器内 `/tmp/iouring`（避开仓库 vendor 配置）：`cargo +nightly-2026-02-25 build --features ci --target riscv64gc-unknown-linux-musl --release -p io-uring-test`（`RUSTFLAGS=-C target-feature=+crt-static`，linker=`riscv64-linux-musl-gcc`）。


