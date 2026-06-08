# mbarrier API 文档

## 概述

`mbarrier` 是一个跨平台的 Rust 内存屏障实现库，参考了 Linux 内核的设计。它为不同的 CPU 架构提供了高效的内存屏障实现。

## 核心函数

### 基础内存屏障

#### `rmb()`

```rust
pub fn rmb()
```

**读内存屏障** - 确保屏障前的所有读操作在屏障后的读操作之前完成。

- **x86/x86_64**: 仅编译器屏障（CPU 保证读顺序）
- **ARM/AArch64**: `dmb ld` / `dsb ld` 指令
- **RISC-V**: `fence r,r` 指令

#### `wmb()`

```rust
pub fn wmb()
```

**写内存屏障** - 确保屏障前的所有写操作在屏障后的写操作之前完成。

- **x86/x86_64**: `sfence` 指令
- **ARM/AArch64**: `dmb st` / `dsb st` 指令  
- **RISC-V**: `fence w,w` 指令

#### `mb()`

```rust
pub fn mb()
```

**通用内存屏障** - 确保屏障前的所有内存操作（读写）在屏障后的内存操作之前完成。

- **x86/x86_64**: `mfence` 指令
- **ARM/AArch64**: `dmb sy` / `dsb sy` 指令
- **RISC-V**: `fence rw,rw` 指令

### SMP 感知屏障

这些函数在多处理器系统上等同于对应的基础屏障，在单处理器系统上仅为编译器屏障。

#### `smp_rmb()`

```rust
pub fn smp_rmb()
```

**SMP 读屏障** - 在 SMP 系统上等同于 `rmb()`，在 UP 系统上为编译器屏障。

#### `smp_wmb()`

```rust
pub fn smp_wmb()
```

**SMP 写屏障** - 在 SMP 系统上等同于 `wmb()`，在 UP 系统上为编译器屏障。

#### `smp_mb()`

```rust
pub fn smp_mb()
```

**SMP 通用屏障** - 在 SMP 系统上等同于 `mb()`，在 UP 系统上为编译器屏障。

### 特殊屏障

#### `read_barrier_depends()`

```rust
pub fn read_barrier_depends()
```

**数据依赖屏障** - 确保依赖读操作的正确顺序。

- **x86/x86_64**: 无操作（数据依赖提供顺序保证）
- **ARM/AArch64**: 无操作（架构保证依赖加载有序）
- **RISC-V**: `fence r,r` 指令（需要显式排序）

#### `smp_read_barrier_depends()`

```rust
pub fn smp_read_barrier_depends()
```

**SMP 数据依赖屏障** - SMP 版本的 `read_barrier_depends()`。

#### `rmb_before_conditional()`

```rust
pub fn rmb_before_conditional()
```

**条件前读屏障** - 在读取可能被其他 CPU 修改的条件前使用的读屏障变体。

## 特性标志

### `smp`（默认启用）

启用 SMP 感知的屏障。当启用时：

- `smp_*()` 函数等同于对应的基础屏障
- 当禁用时，`smp_*()` 函数仅为编译器屏障

### `std`（可选）

启用标准库特性，主要用于示例和基准测试。

## 架构支持

| 架构 | 状态 | 实现方式 |
|------|------|----------|
| x86_64 | ✅ 完全支持 | 内联汇编 + 原子栅栏 |
| x86 | ✅ 完全支持 | 内联汇编 + 原子栅栏 |
| AArch64 | ✅ 完全支持 | 内联汇编 (DSB 指令) |
| ARM | ✅ 完全支持 | 内联汇编 (DMB 指令) |
| RISC-V 64 | ✅ 完全支持 | 内联汇编 (FENCE 指令) |
| RISC-V 32 | ✅ 完全支持 | 内联汇编 (FENCE 指令) |
| 其他 | ⚠️ 通用实现 | Rust 原子栅栏回退 |

## 使用模式

### 生产者-消费者

```rust
use mbarrier::*;

// 生产者
unsafe {
    write_data(42);
    wmb();  // 确保数据写入完成
    set_flag(true);  // 设置就绪标志
}

// 消费者  
unsafe {
    if read_flag() {
        rmb();  // 确保标志读取完成
        let data = read_data();  // 读取数据
    }
}
```

### 自旋锁

```rust
use mbarrier::*;

fn acquire_lock() {
    while !try_lock() {
        spin_loop();
    }
    smp_mb();  // 获取锁后的屏障
}

fn release_lock() {
    smp_mb();  // 释放锁前的屏障
    unlock();
}
```

### 引用计数

```rust
use mbarrier::*;

fn increment_refcount() {
    atomic_increment();
    smp_mb();  // 确保计数增加可见
}

fn decrement_refcount() -> bool {
    smp_mb();  // 减少前的屏障
    let old = atomic_decrement();
    if old == 1 {
        smp_mb();  // 最后引用的屏障
        true  // 可以释放
    } else {
        false
    }
}
```

## 性能考虑

### 相对开销（基于 x86_64 测试）

- `read_barrier_depends()`: 0% 开销（无操作）
- `wmb()`: ~12% 开销
- `rmb()`: ~32% 开销  
- `mb()`: ~185% 开销（最昂贵）

### 建议

1. 仅在需要时使用屏障
2. 优先使用特定类型的屏障（`rmb()`, `wmb()`）而非通用屏障（`mb()`）
3. 在单处理器系统上考虑使用 SMP 版本
4. 在性能关键代码中测量实际影响

## 安全性

- 所有屏障函数都是内联的，提供零开销抽象
- 使用 `unsafe` 内联汇编，但接口是安全的
- 基于 Linux 内核的验证实现
- 遵循各架构的内存模型规范

## 兼容性

- **Rust 版本**: 需要 Rust 2024 edition
- **no_std**: 完全支持无标准库环境
- **目标平台**: 支持所有主流架构
- **内核**: 适用于内核和用户空间代码

## 示例代码

参见：

- `examples/usage.rs` - 基本使用示例
- `examples/performance.rs` - 性能比较测试
- `benches/benchmark.rs` - 详细基准测试
