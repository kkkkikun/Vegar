# 测试运行时间控制

## 概述

`src/init_oscomp.sh` 在编译阶段嵌入内核，启动后依次运行各测试套件。为了防止单测卡死或整体超时，脚本内置了多层时间控制机制。

## 环境变量一览

所有时间控制参数均可通过环境变量在 `make` 前指定，无需修改代码：

| 变量名 | 默认值 | 作用 |
|--------|--------|------|
| `OSCOMP_PER_CASE_TIMEOUT` | `30` | 单个 LTP 测例最大运行秒数 |
| `OSCOMP_LTP_TOTAL_TIMEOUT` | `900` | 预留（当前由逐测例 timeout 兜底） |
| `OSCOMP_TEST_TOTAL_BUDGET` | `3600` | 预留（评测系统通过外部 QEMU 超时控制） |
| `LTP_CASES` | (空) | 指定只运行的部分 LTP 测例 |

### 使用示例

```bash
# 编译时设置更激进的单测超时（测例多时可以更快跳过挂起的）
make all OSCOMP_PER_CASE_TIMEOUT=15

# 只跑几个指定 LTP 测例进行快速回归
make ARCH=riscv64 build LTP_CASES="access01 fork01 mmap02"

# 组合使用
make ARCH=loongarch64 build LTP_CASES="msync04 msync05" OSCOMP_PER_CASE_TIMEOUT=60
```

## 多层时间控制架构

```
评测系统 QEMU 总超时（外部，不可控）
  └── 每运行时（musl/glibc）
        ├── cleanup_background（清残留进程）
        ├── basic  ─── busybox timeout 60s
        ├── busybox ─── busybox timeout 60s
        ├── lua     ─── busybox timeout 60s
        ├── ...     ─── busybox timeout Ns
        ├── LTP ─────── 每个 case: busybox timeout $OSCOMP_PER_CASE_TIMEOUT s
        │               ├── abort01  ── timeout 30s（默认）
        │               ├── access01 ── timeout 30s（默认）
        │               └── ...（约 200 个测例）
        └── cleanup_background（清残留进程）
```

### 第一层：测例级 timeout

每个 LTP 测例由 `busybox timeout` 包裹，超时后自动 SIGKILL。防止：
- 测例调用不存在的系统调用导致死循环
- 测例所需资源未就绪导致无限等待
- 测例间死锁

### 第二层：测试套件级 timeout

非 LTP 测试（basic、busybox、iperf 等）使用 `run_test()` 函数，内置套件级 `timeout`。

### 第三层：进程残留清理

`cleanup_background()` 在关键测试切换点扫描 `/proc` 并强制清理：
- `iperf3` — 网络测试 daemon
- `netserver` — netperf daemon
- `hackbench` — 调度测试 daemon

残留进程会导致：端口被占用、QEMU 无法正常退出、后续测试环境被污染。

### 第四层：LTP 白名单 + LTP_CASES 覆盖

默认运行精心筛选的白名单（约 200 个测例），排除已知会挂起的 cgroup/ftrace/memory 等组。

设置 `LTP_CASES` 环境变量可精确控制只跑哪些测例，适用于：
- 快速回归验证某个修复
- 在接近时间上限时只跑关键测例
- 调试单测失败

## 调优指南

### 场景一：评测超时

```
症状：官方评测显示"评测时间过长"
排查：
  1. 检查是否某个测例超时 -> 减小 OSCOMP_PER_CASE_TIMEOUT
  2. 检查 LTP case 总数 -> 使用 LTP_CASES 精选高价值测例
  3. 检查残留进程 -> 确认 cleanup_background 被调用
```

### 场景二：单测频繁 TIMEOUT

```
症状：大量 LTP 测例输出 "TIMEOUT LTP CASE xxx"
原因：测例在较慢的 QEMU 环境需要更多时间
修复：增大 OSCOMP_PER_CASE_TIMEOUT，如：
  make all OSCOMP_PER_CASE_TIMEOUT=60
```

### 场景三：快速迭代调试

```bash
# 只跑一个测例，给足时间
make ARCH=riscv64 build LTP_CASES="mmap02" OSCOMP_PER_CASE_TIMEOUT=120

# 只跑 non-LTP 测试，跳过 LTP
#（通过设置空的 LTP test 达到）
```

## 注意事项

1. `OSCOMP_PER_CASE_TIMEOUT` 只影响 LTP 测例，不影响 basic/busybox/lua 等非 LTP 测试组
2. 非 LTP 测试组的超时在 `init_oscomp.sh` 中硬编码（如 `run_test "$runtime" basic 60`），直接修改脚本即可
3. LTP 测例的 `timeout` 值过小会导致大量误报 TIMEOUT（测例来不及跑完就被杀），建议不低于 15 秒
4. `LTP_CASES` 覆盖模式下，LTP 白名单被完全绕过，只跑指定的测例
