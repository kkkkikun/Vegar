#!/bin/bash
# OSComp Quick Evaluation Script for StarryOS
# Usage: ./scripts/oscomp_quick_eval.sh [ARCH=riscv64] [TIMEOUT=300] [IMAGE=auto]
set -euo pipefail

# ── 参数解析 ──
ARCH="${1:-riscv64}"
TIMEOUT="${2:-300}"
IMAGE="${3:-auto}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# ── 自动探测镜像 ──
if [ "$IMAGE" = "auto" ]; then
    for candidate in sdcard-riscv64.img sdcard-rv.img; do
        if [ -f "$REPO_ROOT/$candidate" ]; then
            IMAGE="$REPO_ROOT/$candidate"
            break
        fi
    done
    if [ "$IMAGE" = "auto" ]; then
        echo "[ERROR] No test image found. Place sdcard-riscv64.img in repo root."
        exit 1
    fi
fi

# ── 验证 QEMU 路径 ──
QEMU=""
if command -v qemu-system-riscv64 &>/dev/null; then
    QEMU="qemu-system-riscv64"
elif [ -x "/opt/qemu-bin-10.0.2/bin/qemu-system-riscv64" ]; then
    QEMU="/opt/qemu-bin-10.0.2/bin/qemu-system-riscv64"
else
    echo "[ERROR] qemu-system-riscv64 not found"
    exit 1
fi

echo "============================================"
echo " OSComp Quick Evaluation - StarryOS"
echo "============================================"
echo " ARCH:    $ARCH"
echo " TIMEOUT: ${TIMEOUT}s"
echo " IMAGE:   $IMAGE"
echo " QEMU:    $QEMU"
echo "============================================"

# ── 构建 ──
echo ""
echo "[1/3] Building kernel (ARCH=$ARCH, BUS=mmio)..."
docker exec starry-champion-new bash -c \
    "export PATH=/opt/riscv64-linux-musl-cross/bin:/opt/qemu-bin-10.0.2/bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin && \
     cd /workspace && \
     make ARCH=$ARCH BUS=mmio BLK=y NET=y MEM=2G SMP=2 DISK_IMG=sdcard-riscv64.img build 2>&1 | tail -5"
BUILD_EXIT=$?
if [ $BUILD_EXIT -ne 0 ]; then
    echo "[ERROR] Build failed with exit code $BUILD_EXIT"
    exit $BUILD_EXIT
fi

# ── 准备 kernel-rv ──
KERNEL="$REPO_ROOT/workspace_${ARCH}-qemu-virt.bin"
if [ ! -f "$KERNEL" ]; then
    echo "[ERROR] Kernel binary not found: $KERNEL"
    exit 1
fi
cp "$KERNEL" "$REPO_ROOT/kernel-rv"
KERNEL_SIZE=$(ls -lh "$REPO_ROOT/kernel-rv" | awk '{print $5}')
echo "      Kernel: kernel-rv ($KERNEL_SIZE) <- $(basename $KERNEL)"

# ── 日志路径 ──
LOGDIR="$REPO_ROOT/logs"
mkdir -p "$LOGDIR"
LOGFILE="$LOGDIR/oscomp_quick_eval_${ARCH}.log"
rm -f "$LOGFILE"

# ── QEMU 启动 ──
echo ""
echo "[2/3] Launching QEMU (timeout=${TIMEOUT}s)..."

QEMU_CMD="$QEMU \
  -machine virt \
  -kernel $REPO_ROOT/kernel-rv \
  -m 2G \
  -nographic \
  -smp 2 \
  -bios default \
  -drive file=$IMAGE,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -no-reboot \
  -device virtio-net-device,netdev=net \
  -netdev user,id=net \
  -rtc base=utc"

echo "      CMD: $QEMU_CMD"
echo "" > "$LOGFILE"

# 运行 QEMU，带 timeout
set +e
timeout "$TIMEOUT" $QEMU_CMD 2>&1 | tee "$LOGFILE"
QEMU_EXIT=$?
set -e

echo ""
echo "[3/3] QEMU exited with code: $QEMU_EXIT"
echo ""

# ── 结果摘要 ──
echo "============================================"
echo " RESULTS"
echo "============================================"
echo " Log file: $LOGFILE"
echo " Log size: $(ls -lh "$LOGFILE" | awk '{print $5}')"
echo " Exit code: $QEMU_EXIT"

# 快速诊断
echo ""
echo "--- Quick Diagnosis ---"
if grep -qi "panic\|abort\|error.*kernel" "$LOGFILE" 2>/dev/null; then
    echo " [!] Kernel panic/error detected"
    grep -i "panic\|abort\|error.*kernel" "$LOGFILE" | head -5
fi
if grep -qi "virtio" "$LOGFILE" 2>/dev/null; then
    echo " [+] VirtIO device detected"
    grep -i "virtio" "$LOGFILE" | head -3
fi
if grep -qi "ext4\|EXT4\|filesystem" "$LOGFILE" 2>/dev/null; then
    echo " [+] Filesystem detected"
    grep -i "ext4\|EXT4\|filesystem" "$LOGFILE" | head -3
fi
if grep -qi "musl\|glibc\|basic" "$LOGFILE" 2>/dev/null; then
    echo " [+] Test programs referenced"
    grep -i "musl\|glibc\|basic" "$LOGFILE" | head -3
fi
if grep -qi "mount\|rootfs\|root_dev" "$LOGFILE" 2>/dev/null; then
    echo " [+] Mount activity detected"
    grep -i "mount\|rootfs\|root_dev" "$LOGFILE" | head -3
fi
if [ "$QEMU_EXIT" -eq 124 ]; then
    echo " [!] QEMU timed out after ${TIMEOUT}s"
elif [ "$QEMU_EXIT" -eq 0 ]; then
    echo " [+] QEMU exited normally"
else
    echo " [!] QEMU exited with code $QEMU_EXIT"
fi

echo "============================================"
echo " Done. Full log: $LOGFILE"
echo "============================================"
exit $QEMU_EXIT
