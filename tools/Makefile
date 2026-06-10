# Makefile to build build tools required for OSComp evaluation
# These tools are needed even in offline environments

CARGO ?= cargo
TOOLS_DIR := $(dir $(realpath $(lastword $(MAKEFILE_LIST))))
RELEASE_DIR := $(TOOLS_DIR)/target/release

TOOLS := axconfig-gen cargo-axplat

# For offline builds, we copy vendor to tools directory
VENDOR_DIR := $(TOOLS_DIR)../vendor
TOOLS_VENDOR_DIR := $(TOOLS_DIR)vendor

.PHONY: all clean $(TOOLS) prepare-vendor

all: prepare-vendor $(TOOLS)

# Prepare vendor directory for tools
prepare-vendor:
	@if [ ! -d $(TOOLS_VENDOR_DIR) ] && [ -d $(VENDOR_DIR) ]; then \
		echo "Copying vendor for tools..."; \
		cp -r $(VENDOR_DIR) $(TOOLS_VENDOR_DIR); \
		mkdir -p $(TOOLS_DIR).cargo; \
		echo '[source.crates-io]' > $(TOOLS_DIR).cargo/config.toml; \
		echo 'replace-with = "vendored-sources"' >> $(TOOLS_DIR).cargo/config.toml; \
		echo '[source.vendored-sources]' >> $(TOOLS_DIR).cargo/config.toml; \
		echo 'directory = "$(TOOLS_DIR)vendor"' >> $(TOOLS_DIR).cargo/config.toml; \
	fi

axconfig-gen:
	@echo "Building axconfig-gen..."
	@mkdir -p $(RELEASE_DIR)
	@cd $(TOOLS_DIR)axconfig-gen && CARGO_TARGET_DIR=$(TOOLS_DIR)/target $(CARGO) build --release --offline 2>/dev/null || \
		cd $(TOOLS_DIR)axconfig-gen && CARGO_TARGET_DIR=$(TOOLS_DIR)/target $(CARGO) build --release
	@cp $(TOOLS_DIR)axconfig-gen/target/release/axconfig-gen $(RELEASE_DIR)/ 2>/dev/null || true

cargo-axplat:
	@echo "Building cargo-axplat..."
	@mkdir -p $(RELEASE_DIR)
	@cd $(TOOLS_DIR)cargo-axplat && CARGO_TARGET_DIR=$(TOOLS_DIR)/target $(CARGO) build --release --offline 2>/dev/null || \
		cd $(TOOLS_DIR)cargo-axplat && CARGO_TARGET_DIR=$(TOOLS_DIR)/target $(CARGO) build --release
	@cp $(TOOLS_DIR)cargo-axplat/target/release/cargo-axplat $(RELEASE_DIR)/ 2>/dev/null || true

clean:
	rm -rf $(TOOLS_DIR)axconfig-gen/target
	rm -rf $(TOOLS_DIR)cargo-axplat/target
	rm -rf $(TOOLS_DIR)target
	rm -rf $(TOOLS_DIR)vendor
	rm -rf $(TOOLS_DIR).cargo
