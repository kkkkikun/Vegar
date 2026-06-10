/*!
ARM architecture memory barrier implementations.

Based on Linux kernel arch/arm/include/asm/barrier.h
*/

use core::arch::asm;

/// ARM read memory barrier implementation.
///
/// Uses DMB (Data Memory Barrier) for ARMv7+ or fallback for older ARM.
#[inline(always)]
pub fn rmb_impl() {
    unsafe {
        // ARMv7+ has DMB instruction
        #[cfg(target_feature = "v7")]
        asm!("dmb ld", options(nostack, preserves_flags));

        // Fallback for older ARM versions
        #[cfg(not(target_feature = "v7"))]
        asm!("mcr p15, 0, {}, c7, c10, 5", in(reg) 0, options(nostack, preserves_flags));
    }
}

/// ARM write memory barrier implementation.
///
/// Uses DMB (Data Memory Barrier) for ARMv7+ or fallback for older ARM.
#[inline(always)]
pub fn wmb_impl() {
    unsafe {
        // ARMv7+ has DMB instruction
        #[cfg(target_feature = "v7")]
        asm!("dmb st", options(nostack, preserves_flags));

        // Fallback for older ARM versions
        #[cfg(not(target_feature = "v7"))]
        asm!("mcr p15, 0, {}, c7, c10, 4", in(reg) 0, options(nostack, preserves_flags));
    }
}

/// ARM general memory barrier implementation.
///
/// Uses DMB (Data Memory Barrier) for ARMv7+ or fallback for older ARM.
#[inline(always)]
pub fn mb_impl() {
    unsafe {
        // ARMv7+ has DMB instruction
        #[cfg(target_feature = "v7")]
        asm!("dmb sy", options(nostack, preserves_flags));

        // Fallback for older ARM versions
        #[cfg(not(target_feature = "v7"))]
        asm!("mcr p15, 0, {}, c7, c10, 5", in(reg) 0, options(nostack, preserves_flags));
    }
}
