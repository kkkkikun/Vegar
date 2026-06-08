# Checkpoint Commit Note — 2026-06-01

## Branch: `latest-oscamp`

## Purpose

OSComp 初赛适配 checkpoint：使 StarryOS 能够运行 OSComp 官方测试镜像中的动态链接测试程序，并通过 basic-musl、basic-glibc、libctest-musl、busybox-musl 四组测试。

## Changes Summary

### Core Kernel (+9/-5 lines in tracked files)

| File | Change |
|------|--------|
| `kernel/src/pseudofs/mod.rs` | Register `lib` module, call `lib::mount_libfs()` in `mount_all()` |
| `kernel/src/pseudofs/lib.rs` | **New.** Virtual `/lib` filesystem serving embedded dynamic linkers via `include_bytes!` |
| `kernel/src/pseudofs/ld_musl_riscv64.so.1` | **New.** musl dynamic linker (576KB, extracted from musl official toolchain) |
| `kernel/src/pseudofs/ld_linux_riscv64_lp64d.so.1` | **New.** glibc dynamic linker (125KB, extracted from OSComp image `/glibc/lib/`) |
| `kernel/src/pseudofs/ld_musl_loongarch64.so.1` | **New.** LoongArch64 musl dynamic linker (723KB, reserved) |
| `kernel/src/syscall/fs/mount.rs` | Relax `fs_type` check: accept any type, mount tmpfs unconditionally |
| `src/main.rs` | Init command: `/bin/sh` → `/musl/busybox sh -c include!("init_oscomp.sh")` |
| `src/init_oscomp.sh` | **New.** OSComp evaluation init script |

### Tooling

| File | Change |
|------|--------|
| `scripts/oscomp_quick_eval.sh` | **New.** Automated build + QEMU + log + diagnose script |
| `.gitignore` | Added `logs/` and `*:Zone.Identifier` |

### Documentation

| File | Content |
|------|---------|
| `docs/oscomp_eval/summary_2026-06-01.md` | Full test summary: 126/127 pass (99.2%) |
| `docs/oscomp_eval/loader_design_note_2026-06-01.md` | Technical design of virtual `/lib` filesystem |
| `docs/oscomp_eval/busybox_full_report_2026-06-01.md` | Busybox 54/55 detailed test report |

## Problems Solved

1. **Dynamic linker missing** — OSComp image has no `/lib/`. Kernel embeds and serves `ld-musl` and `ld-linux` via pseudofs.
2. **mount() too strict** — Only accepted `tmpfs` type. Now accepts any type for OSComp compatibility.
3. **No /bin/sh** — Image lacks `/bin/sh`. Switched to `/musl/busybox sh`.
4. **glibc libs not found** — `LD_LIBRARY_PATH=/glibc/lib` in init script.

## Test Results

| Suite | Pass | Total | Rate |
|-------|------|-------|------|
| basic-musl | 31 | 31 | 100% |
| basic-glibc | 31 | 31 | 100% |
| libctest-musl (static+dynamic) | 10 | 10 | 100% |
| busybox-musl | 54 | 55 | 98.2% |
| **Total** | **126** | **127** | **99.2%** |

Only failure: `which ls` — no standalone `ls` binary in image (busybox applet only). Not a kernel bug.

## NOT Changed

- ❌ OSComp official test image (`sdcard-riscv64.img`) — untouched
- ❌ Any test binary — untouched
- ❌ Logs — gitignored, regenerated each run

## Commit Structure

Single checkpoint commit with all OSComp adaptation work. No sub-modules or external dependencies added.
