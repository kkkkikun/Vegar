#!/bin/bash
# io_uring demo — compile tests, deploy, boot QEMU
# Prerequisite: kernel already built (make ARCH=riscv64 BUS=mmio TEST_MODE=custom build)
# Usage: ./scripts/demo.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

DOCKER_IMG="zhouzhouyi/os-contest:20260510"
MUSL_PATH="/opt/riscv64-linux-musl-cross/bin"
QEMU_PATH="/opt/qemu-bin-10.0.2/bin"
TIMEOUT=${QEMU_TIMEOUT:-120}

# ── Step 1: Compile test binaries ──
echo "=== [1/3] Compiling tests ==="
docker run --rm -v "$(pwd):/workspace" -w /workspace "$DOCKER_IMG" \
  bash -lc "
export PATH=${MUSL_PATH}:${QEMU_PATH}:\$PATH
CC=riscv64-linux-musl-gcc
CFLAGS='-static -O2'
SHIM='tests/liburing-shim/liburing_shim.c'
SHIM_CFLAGS='-static -O2 -Itests/liburing-shim'

for t in io_uring_nop io_uring_pipe io_uring_poll io_uring_file \
         io_uring_bench io_uring_batch io_uring_scale io_uring_getevents \
         io_uring_readv io_uring_send_recv io_uring_accept io_uring_echo_epoll \
         io_uring_echo_epoll_fair; do
    [ -f tests/\$t.c ] && \$CC \$CFLAGS tests/\$t.c -o \$t 2>/dev/null || true
done
for t in io_uring_shim_test io_uring_iodepth io_uring_echo io_uring_vs_thread; do
    [ -f tests/\$t.c ] && \$CC \$SHIM_CFLAGS tests/\$t.c \$SHIM -o \$t 2>/dev/null || true
done
for v in 24; do
    [ -f tests/io_uring_echo_epoll.c ] && \$CC \$SHIM_CFLAGS -DNCLIENTS=\$v tests/io_uring_echo_epoll.c \$SHIM -o io_uring_echo_epoll\$v 2>/dev/null || true
    [ -f tests/io_uring_echo_epoll_fair.c ] && \$CC \$SHIM_CFLAGS -DNCLIENTS=\$v tests/io_uring_echo_epoll_fair.c \$SHIM -o io_uring_echo_epoll_fair\$v 2>/dev/null || true
done
echo 'Compile done.'
"

# ── Step 2: Deploy to sdcard ──
echo "=== [2/3] Deploying to sdcard-rv.img ==="
docker run --rm --privileged -v "$(pwd):/workspace" -w /workspace "$DOCKER_IMG" \
  bash -lc "
export PATH=${MUSL_PATH}:${QEMU_PATH}:\$PATH
mkdir -p /mnt/sd
mount -o loop sdcard-rv.img /mnt/sd
for t in io_uring_nop io_uring_pipe io_uring_poll io_uring_file \
         io_uring_bench io_uring_batch io_uring_scale io_uring_getevents \
         io_uring_readv io_uring_shim_test io_uring_send_recv io_uring_accept \
         io_uring_iodepth io_uring_echo io_uring_echo_epoll \
         io_uring_echo_epoll_fair24 io_uring_echo_epoll24; do
    [ -f \$t ] && cp \$t /mnt/sd/
done
for t in io_uring_setup io_uring_enter poll fsync poll-cancel; do
    src=/workspace/ref/liburing/test/\$t
    [ -f \$src ] && cp \$src /mnt/sd/liburing_\$t
done
sync && umount /mnt/sd
echo 'Deploy done.'
"

# ── Step 3: Boot QEMU ──
echo "=== [3/3] Booting QEMU (timeout=${TIMEOUT}s) ==="
echo ""
docker run --rm --privileged -v "$(pwd):/workspace" -w /workspace "$DOCKER_IMG" \
  bash -lc "
export PATH=${MUSL_PATH}:${QEMU_PATH}:\$PATH
timeout $TIMEOUT qemu-system-riscv64 \
  -machine virt \
  -kernel workspace_riscv64-qemu-virt.bin \
  -m 1G -nographic -smp 1 -bios default \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -device virtio-net-device,netdev=net -netdev user,id=net \
  -no-reboot -rtc base=utc \
  2>&1
" | grep -E "===|PASS|FAIL|SKIP|selftest|====.*=="

echo ""
echo "=== Demo complete ==="
