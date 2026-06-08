/*!
AArch64 (ARM64) architecture memory barrier implementations.

Based on Linux kernel arch/arm64/include/asm/barrier.h
*/

use core::arch::asm;

/// AArch64 read memory barrier implementation.
///
/// Uses DSB (Data Synchronization Barrier) with LD (load) domain.
#[inline(always)]
pub fn rmb_impl() {
    unsafe {
        asm!("dsb ld", options(nostack, preserves_flags));
    }
}

/// AArch64 write memory barrier implementation.
///
/// Uses DSB (Data Synchronization Barrier) with ST (store) domain.
#[inline(always)]
pub fn wmb_impl() {
    unsafe {
        asm!("dsb st", options(nostack, preserves_flags));
    }
}

/// AArch64 general memory barrier implementation.
///
/// Uses DSB (Data Synchronization Barrier) with SY (full system) domain.
#[inline(always)]
pub fn mb_impl() {
    unsafe {
        asm!("dsb sy", options(nostack, preserves_flags));
    }
}
