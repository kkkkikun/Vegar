/*!
# mbarrier - Cross-platform Memory Barriers for Rust

This crate provides memory barrier implementations inspired by the Linux kernel,
offering cross-platform support for various CPU architectures.

Memory barriers are synchronization primitives that prevent certain types of
memory reordering performed by CPUs and compilers.

## Example

```rust
use mbarrier::*;

// Read memory barrier
rmb();

// Write memory barrier
wmb();

// General memory barrier
mb();

// SMP-aware barriers
smp_rmb();
smp_wmb();
smp_mb();
```
*/

#![no_std]
#![deny(missing_docs)]

/// Architecture-specific barrier implementations
mod arch;

pub use arch::*;

/// Read memory barrier.
///
/// Ensures that all memory reads issued before this barrier are completed
/// before any memory reads issued after this barrier.
#[inline(always)]
pub fn rmb() {
    arch::rmb_impl()
}

/// Write memory barrier.
///
/// Ensures that all memory writes issued before this barrier are completed
/// before any memory writes issued after this barrier.
#[inline(always)]
pub fn wmb() {
    arch::wmb_impl()
}

/// General memory barrier.
///
/// Ensures that all memory operations (reads and writes) issued before this
/// barrier are completed before any memory operations issued after this barrier.
#[inline(always)]
pub fn mb() {
    arch::mb_impl()
}

/// SMP read memory barrier.
///
/// On SMP systems, acts as rmb(). On UP systems, acts as a compiler barrier only.
#[inline(always)]
pub fn smp_rmb() {
    #[cfg(feature = "smp")]
    {
        rmb()
    }
    #[cfg(not(feature = "smp"))]
    {
        use core::sync::atomic::{Ordering, compiler_fence};
        compiler_fence(Ordering::Acquire)
    }
}

/// SMP write memory barrier.
///
/// On SMP systems, acts as wmb(). On UP systems, acts as a compiler barrier only.
#[inline(always)]
pub fn smp_wmb() {
    #[cfg(feature = "smp")]
    {
        wmb()
    }
    #[cfg(not(feature = "smp"))]
    {
        use core::sync::atomic::{Ordering, compiler_fence};
        compiler_fence(Ordering::Release)
    }
}

/// SMP general memory barrier.
///
/// On SMP systems, acts as mb(). On UP systems, acts as a compiler barrier only.
#[inline(always)]
pub fn smp_mb() {
    #[cfg(feature = "smp")]
    {
        mb()
    }
    #[cfg(not(feature = "smp"))]
    {
        use core::sync::atomic::{Ordering, compiler_fence};
        compiler_fence(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_barriers_compile() {
        // These tests just ensure the barriers compile and don't panic
        rmb();
        wmb();
        mb();
        smp_rmb();
        smp_wmb();
        smp_mb();
    }
}
