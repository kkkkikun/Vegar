# Build axconfig-gen tool from source (offline capable)
# Vendor directory and .cargo/config.toml are committed alongside this file.

CARGO ?= cargo
TOOLS_ROOT := $(CURDIR)

.PHONY: all clean axconfig-gen

all: axconfig-gen

axconfig-gen:
	@echo "Building axconfig-gen..."
	cd $(TOOLS_ROOT)/axconfig-gen && CARGO_TARGET_DIR=$(TOOLS_ROOT)/target $(CARGO) build --release --offline

clean:
	rm -rf $(TOOLS_ROOT)/target
