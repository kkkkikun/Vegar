# aarch64-cpu-ext

A Rust library providing extended AArch64 CPU utilities and cache management operations.

## Overview

This library extends the functionality of the `aarch64-cpu` crate by providing additional utilities for:

- Cache management operations (invalidate, clean, flush)
- Cache line size detection
- Low-level assembly operations for cache and TLB management
- Register access through re-exported `aarch64-cpu` registers

## Features

- **Cache Operations**: Comprehensive cache management including clean, invalidate, and flush operations
- **Cache Line Size Detection**: Runtime detection of cache line sizes using CTR_EL0 register
- **Translation Table Entries**: Complete TTE64 implementation supporting 4KB/16KB/64KB granules and 48/52-bit addresses
- **Assembly Wrappers**: Low-level assembly instruction wrappers for cache and TLB operations
- **No Standard Library**: `#![no_std]` compatible for embedded and bare-metal environments
- **Register Access**: Full access to AArch64 system registers through re-exported functionality

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
aarch64-cpu-ext = "0.1.0"
```

### Cache Operations

```rust
use aarch64_cpu_ext::cache::{icache_flush_all, cache_line_size, dcache_range, CacheOp};

// Flush instruction cache
icache_flush_all();

// Get cache line size
let line_size = cache_line_size();

// Perform cache operations on memory ranges
let buffer = &mut [0u8; 4096];
dcache_range(CacheOp::Clean, buffer.as_ptr() as usize, buffer.len());
dcache_range(CacheOp::Invalidate, buffer.as_ptr() as usize, buffer.len());
dcache_range(CacheOp::CleanAndInvalidate, buffer.as_ptr() as usize, buffer.len());
```

### Register Access

```rust
use aarch64_cpu_ext::registers::*;

// Access system registers (re-exported from aarch64-cpu)
let current_el = CurrentEL.get();
```

### Translation Table Entries (TTE64)

```rust
use aarch64_cpu_ext::structures::tte::*;

// Create a 4KB granule, 48-bit address table entry
let table_addr = 0x10_0000; // Must be aligned to granule size
let table_tte = TTE4K48::table(table_addr, 1); // attr_index = 1

// Create a block entry with detailed configuration
let block_addr = 0x20_0000;
let config = BlockConfig {
    attr_index: 0,
    access_permissions: access_permissions::RW_EL1,
    shareability: Shareability::InnerShareable,
    executable: false,
    privileged_executable: false,
    contiguous: false,
    not_global: false,
};
let block_tte = TTE4K48::new_block(block_addr, config);

// Different granule sizes and address widths
let tte_16k_48 = TTE16K48::table(0x4000, 0);   // 16KB granule, 48-bit
let tte_64k_52 = TTE64K52::table(0x10000, 2);  // 64KB granule, 52-bit

// Address alignment utilities
let aligned_addr = TTE4K48::align_up(0x12345);
let is_aligned = TTE64K48::is_aligned(0x10000);

// Virtual address index calculation for page table walks
let va = 0x1234_5678_9ABC_DEF0;
let level0_index = TTE4K48::calculate_index(va, 0);
```

#### Supported Configurations

| Granule | Address Width | Type Alias |
|---------|---------------|------------|
| 4KB     | 48-bit       | `TTE4K48`  |
| 4KB     | 52-bit       | `TTE4K52`  |
| 16KB    | 48-bit       | `TTE16K48` |
| 16KB    | 52-bit       | `TTE16K52` |
| 64KB    | 48-bit       | `TTE64K48` |
| 64KB    | 52-bit       | `TTE64K52` |

### Low-level Assembly Operations

```rust
use aarch64_cpu_ext::asm::cache::{dc, ic, CVAC, IALLU};

// Direct assembly instruction wrappers
ic(IALLU);  // Instruction cache invalidate all
dc(CVAC, address);  // Data cache clean by virtual address
```

## Cache Operation Types

- **Clean**: Write dirty cache lines back to memory without invalidating
- **Invalidate**: Mark cache lines as invalid without writing back
- **CleanAndInvalidate**: Write back dirty lines and mark as invalid

## Requirements

- AArch64 target architecture
- Rust 2024 edition
- No standard library (`#![no_std]`)

## Dependencies

- `aarch64-cpu` version 10 - Provides base AArch64 CPU functionality
- `tock-registers` version 0.9 - Register access and manipulation utilities

## Target Architecture

This library is specifically designed for AArch64 (ARM64) architecture and will not compile for other targets.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contributing

Contributions are welcome! Please ensure that any new features maintain compatibility with `#![no_std]` environments and follow the existing code patterns.
