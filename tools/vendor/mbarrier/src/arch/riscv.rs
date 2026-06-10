/*!
RISC-V architecture memory barrier implementations.

Based on Linux kernel arch/riscv/include/asm/barrier.h

Both RISC-V 32-bit and 64-bit share the same memory barrier semantics:
- Relaxed memory model requiring explicit ordering
- Uses FENCE instruction with r,r / w,w / rw,rw ordering
- Same instruction set for memory barriers
*/

use core::arch::asm;

/// RISC-V read memory barrier implementation.
///
/// Uses FENCE instruction with r,r (read-read) ordering.
#[inline(always)]
pub fn rmb_impl() {
    unsafe {
        asm!("fence r,r", options(nostack, preserves_flags));
    }
}

/// RISC-V write memory barrier implementation.
///
/// Uses FENCE instruction with w,w (write-write) ordering.
#[inline(always)]
pub fn wmb_impl() {
    unsafe {
        asm!("fence w,w", options(nostack, preserves_flags));
    }
}

/// RISC-V general memory barrier implementation.
///
/// Uses FENCE instruction with rw,rw (read-write, read-write) ordering.
#[inline(always)]
pub fn mb_impl() {
    unsafe {
        asm!("fence rw,rw", options(nostack, preserves_flags));
    }
}
