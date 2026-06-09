/*!
x86/x86_64 architecture memory barrier implementations.

Based on Linux kernel arch/x86/include/asm/barrier.h

Both x86 and x86_64 share the same memory barrier semantics:
- Strong memory ordering model
- Reads are not reordered with other reads
- Writes are not reordered with other writes
- Uses mfence/sfence for full barriers
*/

use core::arch::asm;
use core::sync::atomic::{Ordering, fence};

/// x86/x86_64 read memory barrier implementation.
///
/// On x86/x86_64, reads are not reordered with other reads,
/// so rmb() is just a compiler barrier.
#[inline(always)]
pub fn rmb_impl() {
    fence(Ordering::Acquire);
}

/// x86/x86_64 write memory barrier implementation.
///
/// On x86/x86_64, writes are not reordered with other writes,
/// but we need sfence for certain cases (streaming stores, etc).
#[inline(always)]
pub fn wmb_impl() {
    unsafe {
        asm!("sfence", options(nostack, preserves_flags));
    }
}

/// x86/x86_64 general memory barrier implementation.
///
/// mfence provides a full memory barrier on x86/x86_64.
#[inline(always)]
pub fn mb_impl() {
    unsafe {
        asm!("mfence", options(nostack, preserves_flags));
    }
}
