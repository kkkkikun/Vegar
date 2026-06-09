/*!
Generic memory barrier implementations for unsupported architectures.

Provides fallback implementations using Rust's atomic fencing.
*/

use core::sync::atomic::{Ordering, fence};

/// Generic read memory barrier implementation.
///
/// Uses Rust's atomic fence with Acquire ordering.
#[inline(always)]
pub fn rmb_impl() {
    fence(Ordering::Acquire);
}

/// Generic write memory barrier implementation.
///
/// Uses Rust's atomic fence with Release ordering.
#[inline(always)]
pub fn wmb_impl() {
    fence(Ordering::Release);
}

/// Generic memory barrier implementation.
///
/// Uses Rust's atomic fence with SeqCst ordering.
#[inline(always)]
pub fn mb_impl() {
    fence(Ordering::SeqCst);
}
