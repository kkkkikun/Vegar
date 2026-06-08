# VegarOS

**VegarOS - 基于 StarryOS 极限异步化改造的 OS 内核**

VegarOS 名称源自 StarryOS，是基于 StarryOS 进行极限异步化改造的操作系统内核，专为操作系统竞赛设计优化。

[![GitHub Stars](https://img.shields.io/github/stars/Starry-OS/StarryOS?style=for-the-badge)](https://github.com/Starry-OS/StarryOS/stargazers)
[![GitHub Forks](https://img.shields.io/github/forks/Starry-OS/StarryOS?style=for-the-badge)](https://github.com/Starry-OS/StarryOS/network)
[![GitHub License](https://img.shields.io/github/license/Starry-OS/StarryOS?style=for-the-badge)](https://github.com/Starry-OS/StarryOS/blob/main/LICENSE)
[![Build status](https://img.shields.io/github/check-runs/Starry-OS/StarryOS/main?style=for-the-badge)](https://github.com/Starry-OS/StarryOS/actions)

## 支持的架构

- [x] RISC-V 64
- [x] LoongArch64
- [x] AArch64
- [ ] x86_64（开发中）

## 当前进展

### 已完成功能

- ✅ 多架构支持（RISC-V、LoongArch64、AArch64）
- ✅ 基础系统调用实现
- ✅ 动态链接器注入机制
- ✅ OSComp 测试框架集成
- ✅ 测试脚本与启动流程优化
- ✅ LTP 测试套件白名单管理
- ✅ 可选测试组模式（支持 LTP/LMBench 等单独测试）
- ✅ LoongArch64 兼容性修复

### 测试支持

VegarOS 支持以下测试组：

- **基础测试**：basic、busybox、lua
- **C 库测试**：libctest、libcbench
- **性能测试**：cyclictest、unixbench、iozone、lmbench
- **网络测试**：iperf、netperf
- **系统调用测试**：LTP（Linux Test Project）

### 最近更新

- 添加 `ltp-only` 和 `custom` 编译特性，支持灵活的测试模式
- 修复 LoongArch64 上 `mkdir -p` 参数解析问题
- 优化 waitpid EINTR 处理，解决 LTP abort01 测试失败
- 完善 LTP 测试白名单，避免卡在不支持的测试用例上
- 实现 poweroff 系统调用，支持正常关机

## 快速开始

### 1. 克隆仓库

```bash
git clone --recursive https://github.com/Starry-OS/StarryOS.git
cd StarryOS
```

如果已经克隆但没有使用 `--recursive` 选项：

```bash
cd StarryOS
git submodule update --init --recursive
```

### 2. 安装依赖

#### A. 使用 Docker（推荐）

我们提供预装的 Docker 镜像，包含所有所需依赖。

中国大陆用户可使用优化镜像：

```bash
docker pull docker.cnb.cool/starry-os/arceos-build
docker run -it --rm -v $(pwd):/workspace -w /workspace docker.cnb.cool/starry-os/arceos-build
```

其他用户可使用 GitHub 镜像：

```bash
docker pull ghcr.io/arceos-org/arceos-build
docker run -it --rm -v $(pwd):/workspace -w /workspace ghcr.io/arceos-org/arceos-build
```

**注意**：`--rm` 标志会在退出时销毁容器实例。容器内对挂载卷 `/workspace` 之外的修改将会丢失。

#### B. 手动安装

##### i. 安装系统依赖

以 Debian 为例：

```bash
sudo apt update
sudo apt install -y build-essential cmake clang qemu-system
```

**注意**：在 LoongArch64 上运行需要 QEMU 10 或更高版本。如果发行版中的 QEMU 版本过旧，请考虑从[源码](https://www.qemu.org/download)编译。

##### ii. 安装 Musl 工具链

1. 从 [setup-musl releases](https://github.com/arceos-org/setup-musl/releases/tag/prebuilt) 下载文件
2. 解压到指定路径，例如 `/opt/riscv64-linux-musl-cross`
3. 将 bin 目录添加到 `PATH`：

   ```bash
   export PATH=/opt/riscv64-linux-musl-cross/bin:$PATH
   ```

##### iii. 配置 Rust 工具链

```bash
# 从 https://rustup.rs 安装 rustup

cd StarryOS
cargo -V
```

### 3. 构建内核

```bash
# 完整测试链（提交版本）
make all

# 单独构建某个架构
make ARCH=riscv64 BUS=mmio build
make ARCH=loongarch64 BUS=pci build

# LTP 测试模式
make ltp
make ltp-all

# 自定义测试组模式（编辑 src/init_custom.sh 选择测试）
make custom
```

构建输出：
- `kernel-rv` - RISC-V 内核
- `kernel-la` - LoongArch64 内核

### 4. 运行测试

```bash
# 使用提供的测试镜像
qemu-system-riscv64 -machine virt -cpu rv64 -smp 4 -m 1G \
  -kernel kernel-rv \
  -drive file=<测试镜像路径>,if=none,id=blk0 \
  -device virtio-blk-device,drive=blk0,bus=virtio-mmio-bus.0 \
  -nographic
```

## 未来计划

### 短期目标

- [ ] 修复 sys_mincore 中的 AddrSpace mutex reentrancy 问题
- [ ] 完善 LoongArch64 上的文件系统支持
- [ ] 优化 LTP 测试用例通过率
- [ ] 增强网络测试稳定性

### 中期目标

- [ ] 实现更完善的进程管理机制
- [ ] 优化内存管理性能
- [ ] 增加更多驱动支持
- [ ] 改进测试覆盖率和自动化测试

### 长期目标

- [ ] 实现完整的 POSIX 兼容性
- [ ] 支持 x86_64 架构
- [ ] 建立完善的性能测试框架
- [ ] 实现 SMP 多核优化

## 构建选项

```bash
# 查看所有构建选项
make help

# 常用目标
make build          # 构建当前架构
make run            # 运行当前架构
make clean          # 清理构建产物
make ltp            # 构建 LTP 测试模式
make custom         # 构建自定义测试模式
```

## 贡献指南

如果你对项目感兴趣并希望贡献，请参阅[贡献指南](./CONTRIBUTING.md)。

## 许可证

本项目采用 Apache License 2.0 许可证。所有修改和新贡献均在相同许可下分发。详见 [LICENSE](./LICENSE) 和 [NOTICE](./NOTICE) 文件。

---

## 原始 StarryOS 项目信息

本项目基于 [Starry-OS/StarryOS](https://github.com/Starry-OS/StarryOS) 进行修改，是针对操作系统竞赛的优化版本。
