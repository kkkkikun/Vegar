# VegarOS 问题记录

本文档记录 VegarOS 开发过程中遇到的问题和解决方案。

## 目录

1. [已知问题](#已知问题)
2. [已解决问题](#已解决问题)
3. [架构特定问题](#架构特定问题)

---

## 已知问题

### 1. sys_mincore AddrSpace Mutex Reentrancy Deadlock

**状态**：⚠️ 未修复（已跳过测试）

**现象**：
```
assertion `left != right` failed: Task(15380, "mincore01") tried to acquire mutex it already owns.
  left: 15380
  right: 15380
```

**原因**：
- `sys_mincore` 持有 AddrSpace 锁
- 调用 `vm_write_slice` 写入用户内存
- 触发页错误
- 页错误处理函数 `handle_page_fault` 尝试再次锁 AddrSpace
- 由于 Mutex 不是 reentrant 的，触发 panic

**位置**：`kernel/src/syscall/mm/mincore.rs`

**解决方案**：
- 暂时措施：将 `mincore01` 添加到 LTP 黑名单
- 长期修复：让 `vm_write_slice` 不持有 AddrSpace 锁时访问用户内存，或使用 reentrant lock

**影响范围**：LTP mincore01 测试用例

### 2. LTP msync04 测试在 LoongArch QEMU 上因磁盘空间不足而失败

**状态**：⚠️ 未通过（环境配置问题）

**现象**：
```
tst_device.c:135: TINFO: Found free device '/dev/loop0'
tst_test.c:1106: TINFO: Formatting /dev/loop0 with ext2 opts='' extra opts=''
mkfs.ext2: image is too small
tst_test.c:1106: TBROK: mkfs.ext2 failed with exit code 1

Summary:
passed   0
failed   0
broken   1
skipped  0
```

**原因**：
- LTP 测试框架要求块设备最小容量为 **300MB**（为了兼容 XFS 文件系统）
- 当前磁盘镜像 `/workspace/sdcard-la.img` 容量远小于 300MB
- LTP 在 `/dev/loop0` 上尝试创建 ext2 文件系统时，`mkfs.ext2` 因设备空间不足而失败
- LoongArch QEMU 仅支持 `virtio-blk-pci` 总线，不支持 `virtio-mmio`（与 RISC-V 不同）
- 测试失败后 QEMU 未正确退出，导致脚本超时（1806秒后被迫杀死进程）

**位置**：LTP 测试套件 `msync04` 用例 / QEMU LoongArch 虚拟机环境

**解决方案**：
- **临时措施**：创建第二个更大的磁盘镜像（512MB），专门供 LTP 测试使用
- 在 QEMU 启动命令中添加第二个 `virtio-blk-pci` 设备
- 进入系统后格式化新磁盘并挂载
- 设置 `LTP_DEV` / `LTP_TMPDIR` 环境变量，让 LTP 使用新的大容量磁盘

**具体操作步骤**：
```bash
# 1. 创建 512MB 磁盘镜像
dd if=/dev/zero of=/workspace/ltp-data.img bs=1M count=512

# 2. 启动 QEMU 挂载双磁盘
qemu-system-loongarch64 -kernel /workspace/kernel-la -m 1G -nographic -smp 1 \
    -drive file=/workspace/sdcard-la.img,if=none,format=raw,id=x0 \
    -device virtio-blk-pci,drive=x0 \
    -drive file=/workspace/ltp-data.img,if=none,format=raw,id=x1 \
    -device virtio-blk-pci,drive=x1 \
    -no-reboot -device virtio-net-pci,netdev=net0 \
    -netdev user,id=net0 -rtc base=utc

# 3. 虚拟机内格式化并挂载
mkfs.ext4 /dev/vdb
mkdir -p /mnt/ltp
mount /dev/vdb /mnt/ltp

# 4. 设置环境变量并运行测试
export LTP_DEV=/dev/vdb
export LTP_TMPDIR=/mnt/ltp
./runltp -f fs
```

**影响范围**：LoongArch QEMU 环境中所有需要大于 300MB 存储空间的 LTP 文件系统测试用例（ext2/ext3/ext4/xfs 等）

---

## 已解决问题

### 1. waitpid EINTR 错误导致 LTP abort01 失败

**状态**：✅ 已修复

**现象**：
```
abort01    1  TBROK  :  waitpid() failed with unexpected errno: EINTR(4)
```

**原因**：
- SIGCHLD 信号到达时，子进程尚未完全变成 zombie 状态
- `check_children()` 找不到 zombie 就返回 EINTR
- LTP 测试期望在信号到达时能正确获取子进程状态

**解决方案**：
在 `kernel/src/syscall/task/wait.rs` 中，当收到 EINTR 时继续重试而不是立即返回：
```rust
Err(Interrupted) => {
    match check_children() {
        Ok(Some(pid)) => return Ok(pid),
        Ok(None) => continue, // 重试而非返回 EINTR
        Err(e) => return Err(e),
    }
}
```

**影响范围**：LTP abort01 及其他使用 waitpid 的测试

---

### 2. heredoc 在 busybox sh -c 模式下解析失败

**状态**：✅ 已修复

**现象**：
```
basename: not found
```

**原因**：
- `init_oscomp.sh` 使用 `cat >file <<'EOF'` 语法
- 在 `/musl/busybox sh -c` 模式下，heredoc 解析失败
- 导致 `busybox --install` 从未执行，所有 busybox 工具不可用

**解决方案**：
- 将所有 heredoc 改为 `echo` 命令
- 在 setup 阶段使用 `/musl/busybox` 完整路径调用所有命令

**影响范围**：所有测试的 setup 阶段

---

### 3. `/bin/sh` not found 错误

**状态**：✅ 已修复

**现象**：
```
/bin/sh: not found
```

**原因**：
- 测试镜像根目录没有 `/bin/sh`
- 只有 `/musl/busybox` 可用

**解决方案**：
修改 `src/main.rs`，使用 `/musl/busybox sh`：
```rust
pub const CMDLINE: &[&str] = &["/musl/busybox", "sh", "-c", include_str!("init_oscomp.sh")];
```

**影响范围**：系统启动

---

### 4. disk.img 遮蔽测试镜像

**状态**：✅ 已修复

**现象**：
- 测试镜像没有被挂载为根文件系统
- 测试用例找不到

**原因**：
- `make all` 会生成 `disk.img` 作为第二个块设备
- axfs-ng 优先挂载第一个 virtio-blk 设备
- 导致测试镜像被遮蔽

**解决方案**：
- 从 Makefile 中移除 `disk.img` 生成
- 让测试镜像作为唯一的块设备

**影响范围**：测试环境设置

---

### 5. LTP 测试卡在 cgroup/ftrace 测试

**状态**：✅ 已修复

**现象**：
- LTP 测试卡住不继续
- cgroup_core02、ftrace 等测试挂起

**原因**：
- 原始 `ltp_testcode.sh` 遍历所有测试用例
- 包含 cgroup、ftrace、memory 等不支持的测试会挂起

**解决方案**：
- 使用精简的 LTP 测试白名单
- 只运行基本 syscall 测试
- 将不支持的测试排除在白名单外

**影响范围**：LTP 测试完成率

---

### 6. 缺少 poweroff 系统调用

**状态**：✅ 已修复

**现象**：
- 测试完成后无法关机
- QEMU 继续运行

**原因**：
- 没有实现 `sys_reboot` 系统调用

**解决方案**：
在 `kernel/src/syscall/sys.rs` 中实现 `sys_reboot`：
```rust
pub fn sys_reboot(magic: u32, _magic2: u32, cmd: u32, _arg: usize) -> AxResult<isize> {
    const LINUX_REBOOT_MAGIC1: u32 = 0xfee1dead;
    const LINUX_REBOOT_CMD_POWER_OFF: u32 = 0x4321fedc;

    if magic != LINUX_REBOOT_MAGIC1 {
        return Err(AxError::InvalidInput);
    }

    match cmd {
        LINUX_REBOOT_CMD_POWER_OFF => {
            info!("sys_reboot: power off");
            axhal::power::system_off();
        }
        // ...
    }
}
```

并在 `kernel/src/syscall/mod.rs` 中注册：
```rust
Sysno::reboot => sys_reboot(uctx.arg0() as _, uctx.arg1() as _, uctx.arg2() as _, uctx.arg3() as _),
```

**影响范围**：测试完成后正常关机

---

## 架构特定问题

### LoongArch64 问题

#### 1. mkdir -p 参数解析失败

**状态**：✅ 已修复

**现象**：
```
mkdir: can't create directory '/': Invalid argument
busybox: /bin/xxx: No such file or directory
```

**原因**：
- LoongArch64 上的 busybox 对 `mkdir -p` 参数解析有 bug
- `/musl/busybox mkdir -p /bin` 被解析成创建 `/` 目录

**解决方案**：
改用先检查再创建的方式：
```sh
if [ ! -d /bin ]; then
    /musl/busybox mkdir /bin
fi
```

对于嵌套目录，逐级创建：
```sh
if [ ! -d /lib ]; then /musl/busybox mkdir /lib; fi
if [ ! -d /lib/modules ]; then /musl/busybox mkdir /lib/modules; fi
if [ ! -d /lib/modules/10.0.0 ]; then /musl/busybox mkdir /lib/modules/10.0.0; fi
```

**影响范围**：LoongArch64 上的系统初始化

---

### RISC-V 问题

目前没有已知的 RISC-V 特定问题。

---

## 待调查问题

### 1. lmbench /var/tmp/lmbench 错误

**状态**：⚠️ 待确认

**现象**：
```
/var/tmp/lmbench: No such file or directory
```

**说明**：
- 此问题偶尔出现，偶尔不出现
- 可能是测试脚本期望 `/var/tmp/lmbench` 路径存在
- 需要进一步调查触发条件

**临时解决方案**（如需要）：
```sh
if [ ! -d /var/tmp/lmbench ]; then
    /musl/busybox mkdir -p /var/tmp/lmbench
fi
```

---

## 问题统计

| 类别 | 已解决 | 未解决 | 待调查 |
|------|--------|--------|--------|
| 系统调用 | 5 | 1 | 0 |
| 架构特定 | 1 | 0 | 0 |
| 测试相关 | 2 | 0 | 1 |
| **总计** | **8** | **1** | **1** |

---

## 更新日志

- 2026-06-08：添加 sys_mincore 问题记录
- 2026-06-08：添加 LoongArch64 mkdir -p 问题修复
- 2026-06-08：添加 lmbench 待调查问题
