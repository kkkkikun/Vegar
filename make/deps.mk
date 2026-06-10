# Build dependencies for OSComp evaluation environment
# In offline environments, we build tools from included source code

# Path to tools directory
TOOLS_DIR := $(CURDIR)/../tools
TOOLS_BIN_DIR := $(TOOLS_DIR)/target/release

# Add tools bin directory to PATH
export PATH := $(TOOLS_BIN_DIR):$(PATH)

# We only need axconfig-gen now (cargo-axplat is not required with direct vendor paths)
# Check if tool exists, if not build it
BUILD_TOOL_SCRIPT := $(CURDIR)/../scripts/ensure-tools.sh

ifeq ($(wildcard $(TOOLS_BIN_DIR)/axconfig-gen),)
  # Tool doesn't exist, ensure it's built
  $(if $(shell $(BUILD_TOOL_SCRIPT) 2>&1),,\
    $(info Tool build completed),\
    $(error Failed to build axconfig-gen. Please check tools/ directory))
endif

# Cargo binutils (optional, still try to install if network available)
ifeq ($(shell cargo install --list 2>/dev/null | grep cargo-binutils),)
  $(info Installing cargo-binutils...)
  $(shell cargo install cargo-binutils 2>&1 | grep -v "^warning:" | grep -v "^   " || true)
endif
