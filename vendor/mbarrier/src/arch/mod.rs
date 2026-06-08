/*!
Architecture-specific memory barrier implementations.

This module provides platform-specific implementations of memory barriers,
following the patterns used in the Linux kernel.
*/

// Import architecture-specific implementations
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub use x86::*;

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "aarch64")]
pub use aarch64::*;

#[cfg(target_arch = "arm")]
mod arm;
#[cfg(target_arch = "arm")]
pub use arm::*;

#[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
mod riscv;
#[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
pub use riscv::*;

// Fallback implementation for unsupported architectures
#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "x86",
    target_arch = "aarch64",
    target_arch = "arm",
    target_arch = "riscv64",
    target_arch = "riscv32"
)))]
mod generic;

#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "x86",
    target_arch = "aarch64",
    target_arch = "arm",
    target_arch = "riscv64",
    target_arch = "riscv32"
)))]
pub use generic::*;
