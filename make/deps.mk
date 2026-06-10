# Build dependencies for OSComp evaluation environment
# Tools must be available before platform.mk is included

# Path to tools directory
TOOLS_DIR := $(CURDIR)/../tools
TOOLS_BIN_DIR := $(TOOLS_DIR)/target/release

# Add tools bin directory to PATH
export PATH := $(TOOLS_BIN_DIR):$(PATH)

# Ensure axconfig-gen is built BEFORE platform.mk is included
# This is checked at Makefile parse time, so we build synchronously
ifeq ($(wildcard $(TOOLS_BIN_DIR)/axconfig-gen),)
  $(info Building axconfig-gen tool...)
  $(shell $(MAKE) -C $(TOOLS_DIR) axconfig-gen >/dev/null 2>&1 || $(MAKE) -C $(TOOLS_DIR) axconfig-gen)
  $(if $(wildcard $(TOOLS_BIN_DIR)/axconfig-gen),,\
    $(info axconfig-gen: OK),\
    $(error Failed to build axconfig-gen. Please check tools/ directory))
endif

# Cargo binutils check (pre-installed in Docker image, skip if not available)
ifeq ($(shell cargo install --list 2>/dev/null | grep cargo-binutils),)
  $(info Note: cargo-binutils not found. Object copy/size tools fall back to system versions.)
endif
