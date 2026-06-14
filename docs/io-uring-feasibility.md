# io_uring 在 StarryOS 上的可行性报告

> 分支 `ax-pci`，RISC-V（QEMU `virt`）。结论：**可行，两大硬性前提均已具备**，无架构级阻塞。

---

## 结论先行

io_uring 的核心机制是「**用户态与内核共享的 SQ/CQ 环形队列 + 完成驱动**」。它对宿主 OS 提出两个硬要求，我们对 StarryOS 逐一做了源码级核实：

| 硬要求 | StarryOS 现状 | 裁决 |
|---|---|---|
| **① kernel↔user 共享内存**（环缓冲双方可读写） | `SharedPages` + `MAP_SHARED` 匿名映射已现成，且 System V `shmget/shmat` 已在生产中使用同一路径 | ✅ **已具备（EASY）** |
| **② 异步 I/O 后端可被内核 worker 复用** | `Pollable`/`poll_io` 自包含、不绑定任务，pipe/socket 现在就是从调用任务 `block_on(poll_io(...))` 阻塞 | ✅ **成立（FEASIBLE）** |
| 新增 3 个系统调用的接线 | `io_uring_setup` 现为 ENOSYS 空壳；照 `epoll_create1`/eventfd 模式接即可 | ✅ **顺（STRAIGHTFORWARD）** |

---

## 风险一：kernel↔user 共享内存（最关键，已核实）

### 原语：`SharedPages` —— 内核持物理页、用户映射同一份

`kernel/src/mm/aspace/backend/shared.rs:11`

```rust
pub struct SharedPages {
    pub phys_pages: Vec<PhysAddr>,   // 内核持有物理页句柄
    pub size: PageSize,
}
impl SharedPages {
    pub fn new(size: usize, page_size: PageSize) -> AxResult<Self> {
        let num_pages = divide_page(size, page_size);
        let mut result = Self { phys_pages: Vec::with_capacity(num_pages), size: page_size };
        for _ in 0..num_pages {
            result.phys_pages.push(alloc_frame(true, page_size)?);   // 分配物理页
        }
        Ok(result)
    }
}
```

内核侧访问：直接 `phys_to_virt(phys_pages[i])` 读写（cow.rs:103/147、backend/mod.rs:42 都这么用）。
用户侧访问：`SharedBackend::map` 把**同一批物理页**映射进目标用户地址空间——`shared.rs:76`：

```rust
fn map(&self, range: VirtAddrRange, flags: MappingFlags, pt: &mut PageTableCursor) -> AxResult {
    for (vaddr, paddr) in pages_in(range, self.pages.size)?.zip(self.pages_starting_from(range.start)) {
        pt.map(vaddr, *paddr, self.pages.size, flags)?;   // 同一物理页 → 用户虚址
    }
    Ok(())
}
```

⇒ 内核与用户访问的是**同一块物理内存**，正是 io_uring 环所需的精确语义。

### `sys_mmap` 的 `MAP_SHARED` 匿名路径已现成

`kernel/src/syscall/mm/mmap.rs:182`（`fd == -1` 时走 `else` 分支）：

```rust
let backend = match map_type {
    MmapFlags::SHARED | MmapFlags::SHARED_VALIDATE => {
        if let Some(file) = file { /* …文件/设备后端… */ }
        else {
            Backend::new_shared(start, Arc::new(SharedPages::new(length, PageSize::Size4K)?))
            // ↑ 匿名共享：直接拿到内核/用户双访问的页
        }
    }
    MmapFlags::PRIVATE => { /* … */ }
};
```

### mmap-the-fd 模型也成立（Linux io_uring 风格）

`io_uring_setup` 返回一个 fd，用户 `mmap` 该 fd 取环。`sys_mmap` 的文件/设备分支 `mmap.rs:203` 已支持 `device.mmap() → DeviceMmap::Physical(range)`：

```rust
match device.mmap() {
    DeviceMmap::Physical(mut range) => {
        range.start += offset;
        Backend::new_linear(start.as_usize() as isize - range.start.as_usize() as isize)
    }
    // …
}
```

io_uring 的 `FileLike`/`Device` 返回指向自身 `SharedPages` 的 `Physical` range 即可被 mmap。

### 最强佐证

System V 共享内存 `shmget/shmat`（`kernel/src/syscall/ipc/shm.rs:475-487`）用的就是**同一条 `SharedPages` + `Backend::new_shared` 路径**，已在生产中运行——这个原语不是纸面可行，是已经跑通的。

---

## 风险二：Pollable/poll_io 可被内核 worker 复用

io_uring 需要一个内核 worker 任务：从 SQ 取出 `READ/WRITE` SQE，**异步等待目标 fd 就绪后执行 I/O**，而不阻塞提交它的用户任务。

- `Pollable` trait（`vendor/axpoll/src/lib.rs:58`）只要求 `poll()`/`register()`，**不捕获任何任务上下文**；`register()` 接受任意 `&mut Context`，故任何任务都能注册 waker。
- `poll_io`（`vendor/axtask/src/future/poll.rs:17`）把「同步非阻塞闭包」包成 async，WouldBlock 时注册 waker；它不依赖 `current()` 绑定到发起任务。
- pipe 现状（`kernel/src/file/pipe.rs:123,156`）就是从调用任务 `block_on(poll_io(&self, IoEvents::IN, ...))` 阻塞——把「调用任务」换成 `axtask::spawn` 出来的 worker 任务，对同一 `PollSet` 注册/唤醒完全等价。

⇒ worker 任务循环取 SQE、对目标 fd 的 `Pollable` 跑 `block_on(poll_io(...))` 即可，无机制障碍。

---

## 新增系统调用接线

`io_uring_setup` 现在与 `userfaultfd`/`bpf`/`fsopen` 同列，返回 ENOSYS——`kernel/src/syscall/mod.rs:623`：

```rust
| Sysno::userfaultfd
| Sysno::perf_event_open
| Sysno::io_uring_setup        // ← 现为 sys_dummy_fd（ENOSYS）
| Sysno::bpf
| Sysno::fsopen
```

「建内核对象 + 返回 fd」的标准模式已有现成范例 `sys_epoll_create1`——`kernel/src/syscall/io_mpx/epoll.rs:31`：

```rust
pub fn sys_epoll_create1(flags: u32) -> AxResult<isize> {
    let flags = EpollCreateFlags::from_bits(flags).ok_or(AxError::InvalidInput)?;
    Epoll::new()
        .add_to_fd_table(flags.contains(EpollCreateFlags::CLOEXEC))   // 注册进 FD_TABLE
        .map(|fd| fd as isize)                                        // 返回 fd
}
```

`io_uring_setup` 照此办理：构造 `IoRing`（含 `SharedPages` 环 + worker）→ `add_to_fd_table` → 返 fd。`io_uring_enter`/`io_uring_register` 同理。

---

## 代码索引

| 关注点 | 文件:行 |
|---|---|
| 共享内存原语 `SharedPages` | `kernel/src/mm/aspace/backend/shared.rs:11` |
| 物理页映射进用户态 `SharedBackend::map` | `kernel/src/mm/aspace/backend/shared.rs:76` |
| `Backend::new_shared` 构造 | `kernel/src/mm/aspace/backend/shared.rs:106` |
| `sys_mmap` MAP_SHARED 匿名 | `kernel/src/syscall/mm/mmap.rs:182,230-232` |
| `sys_mmap` 设备 `mmap()` 分支（mmap-the-fd） | `kernel/src/syscall/mm/mmap.rs:203` |
| System V shm 同路径佐证 | `kernel/src/syscall/ipc/shm.rs:475-487` |
| `Pollable` trait | `vendor/axpoll/src/lib.rs:58` |
| `poll_io` 异步包装 | `vendor/axtask/src/future/poll.rs:17` |
| `block_on` 执行器 | `vendor/axtask/src/future/mod.rs:55` |
| pipe 阻塞范例 | `kernel/src/file/pipe.rs:123,156` |
| `io_uring_setup` ENOSYS 桩 | `kernel/src/syscall/mod.rs:623` |
| 「建对象+返 fd」范例 `sys_epoll_create1` | `kernel/src/syscall/io_mpx/epoll.rs:31` |
| `add_to_fd_table` 注册 | `kernel/src/file/mod.rs:173` |

---

## 诚实的保留（实现工作量，非可行性阻塞）

可行性已清，但以下仍是**真正的工程量**，会落在 MVP 早期里程碑里：

- worker **跨任务访问提交进程的 fd 表与用户缓冲**（fd 解析、`UserPtr` 跨地址空间拷贝）——需要设计 per-process 上下文关联。
- io_uring 环的**内存序/屏障**（`sqring`/`cqring` 的 `smp_store_release`/`smp_load_acquire` 语义）与 StarryOS 现有原子设施对接。
- `IORING_OP_POLL_ADD` 与现有 `axpoll`/`epoll` 的就绪检测整合。

这些是「怎么做」的问题，不构成「能不能做」的风险。**裁决：io_uring 可行，可锁定为实践任务方向。**
