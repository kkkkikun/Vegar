# Build dependencies for OSComp evaluation environment
# In offline environments, we build tools from included source code

# Path to tools directory
TOOLS_DIR := $(CURDIR)/../tools
TOOLS_BIN_DIR := $(TOOLS_DIR)/target/release

# Add tools bin directory to PATH
export PATH := $(TOOLS_BIN_DIR):$(PATH)

# We only need axconfig-gen now (cargo-axplat is not required with direct vendor paths)
ifeq ($(wildcard $(TOOLS_BIN_DIR)/axconfig-gen),)
  # axconfig-gen not built yet, build it
  $(info Build tool not found, building axconfig-gen from source...)
  $(shell $(MAKE) -C $(TOOLS_DIR) axconfig-gen > /dev/null 2>&1)
  $(if $(wildcard $(TOOLS_BIN_DIR)/axconfig-gen),,\
    $(info axconfig-gen: OK),\
    $(error Failed to build axconfig-gen. Please check tools/ directory))
endif

# Cargo binutils (optional, still try to install if network available)
ifeq ($(shell cargo install --list 2>/dev/null | grep cargo-binutils),)
  $(info Installing cargo-binutils...)
  $(shell cargo install cargo-binutils 2>&1 | grep -v "^warning:" | grep -v "^   " || true)
endif
