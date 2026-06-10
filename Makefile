# Build Options
export APP := $(PWD)
export ARCH := riscv64
export LOG := warn
export DWARF := y
export MEMTRACK := n
export TEST_MODE := full  # full, ltp, or custom

# QEMU Options
export BLK := y
export NET := y
export VSOCK := n
export MEM := 1G
export ICOUNT := n

# Generated Options
export A := $(PWD)
export NO_AXSTD := y
export AX_LIB := axfeat
export APP_FEATURES := qemu

ifeq ($(MEMTRACK), y)
	APP_FEATURES += starry-api/memtrack
endif

# Add LTP/Custom test mode feature if requested
# Note: These are features defined in src/main.rs, not standard cargo features
ifeq ($(TEST_MODE), ltp)
APP_FEATURES += ltp-only
endif

ifeq ($(TEST_MODE), custom)
APP_FEATURES += custom
endif

# =========================================================================
# Rust toolchain detection for offline evaluation environment.
# The project requires nightly-2026-02-25, but the official Docker image
# contains nightly-2025-05-20 instead. In CI (with network) the correct
# toolchain is installed normally; in the offline eval environment we
# fall back to the Docker's pre-installed nightly.
# =========================================================================
RUSTUP_TOOLCHAIN := $(shell \
  if rustup toolchain list 2>/dev/null | grep -qF "nightly-2026-02-25"; then \
    echo "nightly-2026-02-25"; \
  elif rustup toolchain list 2>/dev/null | grep -qF "nightly-2025-05-20"; then \
    echo "nightly-2025-05-20"; \
  fi)
export RUSTUP_TOOLCHAIN

# When using the fallback toolchain (nightly-2025-05-20), enable unstabilized
# library features that are required by vendored crates but not yet stable.
# These features ARE stable in the project's default toolchain (nightly-2026-02-25).
ifneq ($(filter nightly-2025-05-20,$(RUSTUP_TOOLCHAIN)),)
  export RUSTFLAGS := -Z crate-attr=feature(unsigned_signed_diff) -Z crate-attr=feature(maybe_uninit_slice)
endif


# =========================================================================
# Fix for hidden directories in competition environment
# The evaluation system filters out hidden files/directories during clone.
# We rename them to non-hidden names before build.
# =========================================================================
.PHONY: prepare-hidden
prepare-hidden:
	@# Setup cargo config for offline build
	@# First, try to restore from 'cargo' directory (non-hidden, survives filtering)
	@if [ -d cargo ] && [ ! -d .cargo ]; then \
		cp -r cargo .cargo; \
	fi
	@# Fallback to cargo-config directory
	@if [ -d cargo-config ] && [ ! -d .cargo ]; then \
		cp -r cargo-config .cargo; \
	fi
	@# Configure vendor for fully offline build
	@if [ -d vendor ]; then \
		mkdir -p .cargo; \
		echo '[source.crates-io]' > .cargo/config.toml; \
		echo 'replace-with = "vendored-sources"' >> .cargo/config.toml; \
		echo '' >> .cargo/config.toml; \
		echo '[source.vendored-sources]' >> .cargo/config.toml; \
		echo 'directory = "vendor"' >> .cargo/config.toml; \
		echo '' >> .cargo/config.toml; \
		echo '[net]' >> .cargo/config.toml; \
		echo 'offline = true' >> .cargo/config.toml; \
		echo 'git-fetch-with-cli = true' >> .cargo/config.toml; \
		echo '' >> .cargo/config.toml; \
		echo '[http]' >> .cargo/config.toml; \
		echo 'check-revoke = false' >> .cargo/config.toml; \
	else \
		if [ -f .cargo/config.toml ]; then \
			rm -f .cargo/config.toml; \
		fi \
	fi
	@# If the project-specified nightly is not available (offline
	@# evaluation environment), use RUSTFLAGS to add feature gates for
	@# unstable APIs that the fallback nightly does not yet stabilize.
	@if ! rustup toolchain list 2>/dev/null | grep -qF "nightly-2026-02-25"; then \
		if rustup toolchain list 2>/dev/null | grep -qF "nightly-2025-05-20"; then \
			echo "Using fallback toolchain (nightly-2025-05-20) — setting RUSTFLAGS for feature compatibility..."; \
		fi; \
	fi
	@# Verify required build targets are installed for the current toolchain.
	@# Uses rustup's native path resolution (handles varying HOME/RUSTUP_HOME).
	@for target in riscv64gc-unknown-none-elf loongarch64-unknown-none-softfloat; do \
		tc_sysroot=$$(rustup run $(RUSTUP_TOOLCHAIN) rustc --print sysroot 2>/dev/null); \
		if [ ! -d "$$tc_sysroot/lib/rustlib/$$target" ]; then \
			echo "ERROR: Required target '$$target' not installed for $(RUSTUP_TOOLCHAIN)."; \
			exit 1; \
		fi \
	done
	@# Ensure build tools are available
	@if [ ! -x tools/target/release/axconfig-gen ]; then \
		echo "Building axconfig-gen tool..."; \
		$(MAKE) -C tools axconfig-gen; \
	fi
	@# Clean old config when ARCH changes
	@if [ -f make/.old_config_arch ]; then \
		if [ "$(ARCH)" != "$$(cat make/.old_config_arch 2>/dev/null)" ]; then \
			rm -f .axconfig.toml; \
		fi \
	fi
default: prepare-hidden build

build: prepare-hidden

# === OSComp submission entry: build both arch kernels ===
# Uses bin format since QEMU -kernel loads raw binaries at correct address.
# The test image (provided by competition) is the first block device and gets
# mounted as root by axfs-ng. We do NOT produce disk.img to avoid shadowing it.
all:
	@echo "=== Building OSComp 2026 Submission ==="
	@echo "Building RISC-V kernel (MMIO bus, 1GB memory)..."
	@$(MAKE) ARCH=riscv64 BUS=mmio build
	@echo "Building LoongArch64 kernel (PCI bus, 1GB memory)..."
	@$(MAKE) ARCH=loongarch64 BUS=pci build
	@echo "Copying kernels for submission..."
	@# The output filename depends on the repo name (StarryOS_*, Vegar_*, etc.)
	@# Use glob pattern to find any matching binary
	@_rv=$$(ls *_riscv64-qemu-virt.bin 2>/dev/null | head -1); \
	if [ -n "$$_rv" ]; then \
		cp "$$_rv" kernel-rv; \
	else \
		echo "ERROR: Cannot find RISC-V kernel binary"; \
		exit 1; \
	fi
	@_la=$$(ls *_loongarch64-qemu-virt.bin 2>/dev/null | head -1); \
	if [ -n "$$_la" ]; then \
		cp "$$_la" kernel-la; \
	else \
		echo "ERROR: Cannot find LoongArch64 kernel binary"; \
		exit 1; \
	fi
	@echo "✓ Build complete:"
	@echo "  kernel-rv ($(shell wc -c < kernel-rv 2>/dev/null) bytes)"
	@echo "  kernel-la ($(shell wc -c < kernel-la 2>/dev/null) bytes)"

ROOTFS_URL = https://github.com/Starry-OS/rootfs/releases/download/20260214
ROOTFS_IMG = rootfs-$(ARCH).img

rootfs:
	@if [ ! -f $(ROOTFS_IMG) ]; then \
		echo "Image not found, downloading..."; \
		curl -f -L $(ROOTFS_URL)/$(ROOTFS_IMG).xz -O; \
		xz -d $(ROOTFS_IMG).xz; \
	fi
	@cp $(ROOTFS_IMG) make/disk.img

img:
	@echo -e "\033[33mWARN: The 'img' target is deprecated. Please use 'rootfs' instead.\033[0m"
	@$(MAKE) --no-print-directory rootfs

defconfig: prepare-hidden
	@$(MAKE) -C make $@

justrun clean:
	@$(MAKE) -C make $@

build run debug disasm: defconfig
	@$(MAKE) -C make $@

ci-test:
	./scripts/ci-test.py $(ARCH)

# Aliases
rv:
	$(MAKE) ARCH=riscv64 run

la:
	$(MAKE) ARCH=loongarch64 run

vf2:
	$(MAKE) ARCH=riscv64 APP_FEATURES=vf2 MYPLAT=axplat-riscv64-visionfive2 BUS=mmio build

# === LTP test mode: build kernels with LTP-only test script ===
# Use these targets to quickly iterate on LTP tests without running full test suite
ltp: ltp-rv

ltp-rv:
	@echo "=== Building RISC-V kernel (LTP test mode) ==="
	@$(MAKE) ARCH=riscv64 BUS=mmio TEST_MODE=ltp build
	@if [ -f StarryOS_riscv64-qemu-virt.bin ]; then \
		cp StarryOS_riscv64-qemu-virt.bin kernel-rv; \
	elif [ -f kernel/kernel_riscv64-qemu-virt.bin ]; then \
		cp kernel/kernel_riscv64-qemu-virt.bin kernel-rv; \
	elif [ -f workspace_riscv64-qemu-virt.bin ]; then \
		cp workspace_riscv64-qemu-virt.bin kernel-rv; \
	else \
		echo "ERROR: Cannot find RISC-V kernel binary"; \
		exit 1; \
	fi
	@echo "✓ LTP kernel ready: kernel-rv"

ltp-la:
	@echo "=== Building LoongArch64 kernel (LTP test mode) ==="
	@$(MAKE) ARCH=loongarch64 BUS=pci TEST_MODE=ltp build
	@if [ -f StarryOS_loongarch64-qemu-virt.bin ]; then \
		cp StarryOS_loongarch64-qemu-virt.bin kernel-la; \
	elif [ -f kernel/kernel_loongarch64-qemu-virt.bin ]; then \
		cp kernel/kernel_loongarch64-qemu-virt.bin kernel-la; \
	elif [ -f workspace_loongarch64-qemu-virt.bin ]; then \
		cp workspace_loongarch64-qemu-virt.bin kernel-la; \
	else \
		echo "ERROR: Cannot find LoongArch64 kernel binary"; \
		exit 1; \
	fi
	@echo "✓ LTP kernel ready: kernel-la"

ltp-all:
	@echo "=== Building both kernels (LTP test mode) ==="
	@$(MAKE) ARCH=riscv64 BUS=mmio TEST_MODE=ltp build
	@$(MAKE) ARCH=loongarch64 BUS=pci TEST_MODE=ltp build
	@if [ -f StarryOS_riscv64-qemu-virt.bin ]; then \
		cp StarryOS_riscv64-qemu-virt.bin kernel-rv; \
	elif [ -f kernel/kernel_riscv64-qemu-virt.bin ]; then \
		cp kernel/kernel_riscv64-qemu-virt.bin kernel-rv; \
	elif [ -f workspace_riscv64-qemu-virt.bin ]; then \
		cp workspace_riscv64-qemu-virt.bin kernel-rv; \
	else \
		echo "ERROR: Cannot find RISC-V kernel binary"; \
		exit 1; \
	fi
	@if [ -f StarryOS_loongarch64-qemu-virt.bin ]; then \
		cp StarryOS_loongarch64-qemu-virt.bin kernel-la; \
	elif [ -f kernel/kernel_loongarch64-qemu-virt.bin ]; then \
		cp kernel/kernel_loongarch64-qemu-virt.bin kernel-la; \
	elif [ -f workspace_loongarch64-qemu-virt.bin ]; then \
		cp workspace_loongarch64-qemu-virt.bin kernel-la; \
	else \
		echo "ERROR: Cannot find LoongArch64 kernel binary"; \
		exit 1; \
	fi
	@echo "✓ LTP kernels ready:"
	@echo "  kernel-rv ($(shell wc -c < kernel-rv 2>/dev/null) bytes)"
	@echo "  kernel-la ($(shell wc -c < kernel-la 2>/dev/null) bytes)"

# === Custom test mode: run selected test groups defined in init_custom.sh ===
# Edit TEST_GROUPS at the top of src/init_custom.sh to choose which tests to run
custom: custom-rv

custom-rv:
	@echo "=== Building RISC-V kernel (custom test mode) ==="
	@echo "Edit TEST_GROUPS in src/init_custom.sh to select test groups"
	@$(MAKE) ARCH=riscv64 BUS=mmio TEST_MODE=custom build
	@if [ -f kernel/kernel_riscv64-qemu-virt.bin ]; then \
		cp kernel/kernel_riscv64-qemu-virt.bin kernel-rv; \
	elif [ -f workspace_riscv64-qemu-virt.bin ]; then \
		cp workspace_riscv64-qemu-virt.bin kernel-rv; \
	else \
		echo "ERROR: Cannot find RISC-V kernel binary"; \
		exit 1; \
	fi
	@echo "✓ Custom kernel ready: kernel-rv"

custom-la:
	@echo "=== Building LoongArch64 kernel (custom test mode) ==="
	@echo "Edit TEST_GROUPS in src/init_custom.sh to select test groups"
	@$(MAKE) ARCH=loongarch64 BUS=pci TEST_MODE=custom build
	@if [ -f StarryOS_loongarch64-qemu-virt.bin ]; then \
		cp StarryOS_loongarch64-qemu-virt.bin kernel-la; \
	elif [ -f kernel/kernel_loongarch64-qemu-virt.bin ]; then \
		cp kernel/kernel_loongarch64-qemu-virt.bin kernel-la; \
	elif [ -f workspace_loongarch64-qemu-virt.bin ]; then \
		cp workspace_loongarch64-qemu-virt.bin kernel-la; \
	else \
		echo "ERROR: Cannot find LoongArch64 kernel binary"; \
		exit 1; \
	fi
	@echo "✓ Custom kernel ready: kernel-la"

custom-all:
	@echo "=== Building both kernels (custom test mode) ==="
	@echo "Edit TEST_GROUPS in src/init_custom.sh to select test groups"
	@$(MAKE) ARCH=riscv64 BUS=mmio TEST_MODE=custom build
	@$(MAKE) ARCH=loongarch64 BUS=pci TEST_MODE=custom build
	@if [ -f StarryOS_riscv64-qemu-virt.bin ]; then \
		cp StarryOS_riscv64-qemu-virt.bin kernel-rv; \
	elif [ -f kernel/kernel_riscv64-qemu-virt.bin ]; then \
		cp kernel/kernel_riscv64-qemu-virt.bin kernel-rv; \
	elif [ -f workspace_riscv64-qemu-virt.bin ]; then \
		cp workspace_riscv64-qemu-virt.bin kernel-rv; \
	else \
		echo "ERROR: Cannot find RISC-V kernel binary"; \
		exit 1; \
	fi
	@if [ -f StarryOS_loongarch64-qemu-virt.bin ]; then \
		cp StarryOS_loongarch64-qemu-virt.bin kernel-la; \
	elif [ -f kernel/kernel_loongarch64-qemu-virt.bin ]; then \
		cp kernel/kernel_loongarch64-qemu-virt.bin kernel-la; \
	elif [ -f workspace_loongarch64-qemu-virt.bin ]; then \
		cp workspace_loongarch64-qemu-virt.bin kernel-la; \
	else \
		echo "ERROR: Cannot find LoongArch64 kernel binary"; \
		exit 1; \
	fi
	@echo "✓ Custom kernels ready:"
	@echo "  kernel-rv ($(shell wc -c < kernel-rv 2>/dev/null) bytes)"
	@echo "  kernel-la ($(shell wc -c < kernel-la 2>/dev/null) bytes)"

.PHONY: all build run justrun debug disasm clean ltp ltp-rv ltp-la ltp-all custom custom-rv custom-la custom-all
