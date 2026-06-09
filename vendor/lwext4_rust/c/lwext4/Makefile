
#Release
#Debug
BUILD_TYPE = Release

ifneq ($(shell test -d .git), 0)
GIT_SHORT_HASH:= $(shell git rev-parse --short HEAD)
endif

VERSION_MAJOR = 1
VERSION_MINOR = 0
VERSION_PATCH = 0

VERSION = $(VERSION_MAJOR).$(VERSION_MINOR).$(VERSION_PATCH)-$(GIT_SHORT_HASH)

# Directory for build output. Defaults to the source directory.
# Pass OUT_DIR=<path> to place build artifacts outside the source tree.
OUT_DIR ?= .
SRCDIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))

COMMON_DEFINITIONS =                                      \
	-DCMAKE_BUILD_TYPE=$(BUILD_TYPE)                      \
	-DVERSION_MAJOR=$(VERSION_MAJOR)                      \
	-DVERSION_MINOR=$(VERSION_MINOR)                      \
	-DVERSION_PATCH=$(VERSION_PATCH)                      \
	-DVERSION=$(VERSION)                                  \
	-DLWEXT4_BUILD_SHARED_LIB=OFF \
	-DLWEXT4_ULIBC=$(ULIBC) \
	-DCMAKE_C_COMPILER_WORKS=1 \
	-DCMAKE_INSTALL_PREFIX=./install \

define generate_common
	rm -R -f $(OUT_DIR)/build_$(1)
	mkdir -p $(OUT_DIR)/build_$(1)
	cd $(OUT_DIR)/build_$(1) && cmake -G"Unix Makefiles" \
	$(COMMON_DEFINITIONS)                               \
	$(2)                                                \
	-DCMAKE_TOOLCHAIN_FILE=$(SRCDIR)/toolchain/$(1).cmake $(SRCDIR)
endef

ARCH ?= x86_64
#Output: src/liblwext4.a
musl-generic:
	$(call generate_common,$@)
	cd $(OUT_DIR)/build_$@ && make lwext4
	cp $(OUT_DIR)/build_$@/src/liblwext4.a $(OUT_DIR)/liblwext4-$(ARCH).a

generic:
	$(call generate_common,$@)

cortex-m0:
	$(call generate_common,$@)

cortex-m0+:
	$(call generate_common,$@)

cortex-m3:
	$(call generate_common,$@)

cortex-m4:
	$(call generate_common,$@)

cortex-m4f:
	$(call generate_common,$@)

cortex-m7:
	$(call generate_common,$@)

arm-sim:
	$(call generate_common,$@)

avrxmega7:
	$(call generate_common,$@)

msp430:
	$(call generate_common,$@)

mingw:
	$(call generate_common,$@,-DWIN32=1)

lib_only:
	rm -R -f $(OUT_DIR)/build_lib_only
	mkdir -p $(OUT_DIR)/build_lib_only
	cd $(OUT_DIR)/build_lib_only && cmake $(COMMON_DEFINITIONS) -DLIB_ONLY=TRUE $(SRCDIR)

all:
	generic

clean:
	rm -R -f $(OUT_DIR)/build_*
	rm -R -f $(OUT_DIR)/ext_images

include fs_test.mk
