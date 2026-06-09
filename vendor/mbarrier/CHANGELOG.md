# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2024-07-02

### Added

- Initial implementation of cross-platform memory barriers
- Support for x86, x86_64, ARM, AArch64, RISC-V 32/64 architectures
- Core memory barrier functions:
  - `rmb()` - Read memory barrier
  - `wmb()` - Write memory barrier  
  - `mb()` - General memory barrier
- SMP-aware memory barriers:
  - `smp_rmb()` - SMP read memory barrier
  - `smp_wmb()` - SMP write memory barrier
  - `smp_mb()` - SMP general memory barrier
- Data dependency barriers:
  - `read_barrier_depends()` - Data dependency barrier
  - `smp_read_barrier_depends()` - SMP data dependency barrier
  - `rmb_before_conditional()` - Read barrier before conditional
- Architecture-specific optimizations:
  - x86/x86_64: Uses `mfence`, `sfence`, and compiler barriers
  - ARM/AArch64: Uses `dmb`/`dsb` instructions with appropriate domains
  - RISC-V: Uses `fence` instruction with specific ordering constraints
- Feature flags:
  - `smp` (default) - Enable SMP-aware barriers
  - `std` (optional) - Enable standard library features for examples
- Comprehensive documentation and examples
- Performance benchmarking utilities
- no_std compatibility
- Dual MIT/Apache-2.0 licensing

### Architecture Support Matrix

- ✅ x86/x86_64: Unified implementation with inline assembly
- ✅ AArch64: Full support with DSB instructions
- ✅ ARM: Full support with DMB instructions (ARMv7+ optimized)
- ✅ RISC-V 64: Full support with FENCE instructions
- ✅ RISC-V 32: Full support with FENCE instructions
- ⚠️  Other: Generic fallback using Rust atomic fences

### Examples

- Basic usage example demonstrating all barrier types
- Performance comparison benchmark
- Producer-consumer pattern implementation
- Lock-free data structure examples

### Performance Characteristics (x86_64)

- `read_barrier_depends()`: 0% overhead (no-op)
- `wmb()`: ~12% overhead
- `rmb()`: ~32% overhead
- `mb()`: ~185% overhead

[Unreleased]: https://github.com/yourusername/mbarrier/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yourusername/mbarrier/releases/tag/v0.1.0
