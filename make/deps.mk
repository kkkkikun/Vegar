# Necessary dependencies for the build system

# When installing tools, we need network access even if vendor is configured
# Use CARGO_NET_GIT_FETCH_WITH_CLI=true and --index to bypass vendor-only mode
CARGO_INSTALL_FLAGS := --index https://github.com/rust-lang/crates.io-index

# Tool to parse information about the target package
ifeq ($(shell cargo axplat --version 2>/dev/null),)
  $(info Installing cargo-axplat...)
  $(shell cargo install $(CARGO_INSTALL_FLAGS) cargo-axplat)
endif

# Tool to generate platform configuration files
ifeq ($(shell axconfig-gen --version 2>/dev/null),)
  $(info Installing axconfig-gen...)
  $(shell cargo install $(CARGO_INSTALL_FLAGS) axconfig-gen)
endif

# Cargo binutils
ifeq ($(shell cargo install --list | grep cargo-binutils),)
  $(info Installing cargo-binutils...)
  $(shell cargo install $(CARGO_INSTALL_FLAGS) cargo-binutils)
endif
