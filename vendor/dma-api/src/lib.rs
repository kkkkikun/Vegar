#![cfg_attr(not(test), no_std)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::{ptr::NonNull, sync::atomic::AtomicBool};

mod dma;
mod osal;

#[cfg(feature = "alloc")]
pub use dma::alloc::{pool::*, r#box::DBox, vec::DVec, DError};

pub use dma::slice::{DSlice, DSliceMut};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum Direction {
    ToDevice,
    FromDevice,
    Bidirectional,
}

pub trait Osal {
    /// map virt address to physical address
    fn map(&self, addr: NonNull<u8>, size: usize, direction: Direction) -> u64;

    /// unmap virt address
    fn unmap(&self, addr: NonNull<u8>, size: usize);

    /// write cache back to memory
    fn flush(&self, addr: NonNull<u8>, size: usize) {
        osal::arch::flush(addr, size)
    }

    /// invalidate cache
    fn invalidate(&self, addr: NonNull<u8>, size: usize) {
        osal::arch::invalidate(addr, size)
    }

    /// allocate memory that meets the dma requirement
    ///
    /// # Safety
    /// This function is unsafe because undefined behavior can
    /// result if the caller does not ensure that the returned pointer is
    /// properly handled.
    /// The caller must ensure that the pointer is eventually deallocated
    ///  using the corresponding `dealloc` method, and that the memory is not accessed after being deallocated.
    #[cfg(feature = "alloc")]
    unsafe fn alloc(&self, dma_mask: u64, layout: core::alloc::Layout) -> *mut u8 {
        let _ = dma_mask;
        alloc::alloc::alloc(layout)
    }

    /// deallocate memory
    ///
    /// # Safety
    ///
    /// This function is unsafe because undefined behavior can result if the caller does not ensure that the `ptr` was allocated by a previous call to the `alloc` method with the same `layout`.
    /// The caller must ensure that the memory is not accessed after being deallocated.
    #[cfg(feature = "alloc")]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        alloc::alloc::dealloc(ptr, layout)
    }
}

static mut OSAL: &'static dyn Osal = &osal::NopOsal;
static INIT: AtomicBool = AtomicBool::new(false);

pub fn init(osal: &'static dyn Osal) {
    if INIT.load(core::sync::atomic::Ordering::Acquire) {
        return;
    }

    unsafe {
        OSAL = osal;
    }
    INIT.store(true, core::sync::atomic::Ordering::Release);
}

fn get_osal() -> &'static dyn Osal {
    if !INIT.load(core::sync::atomic::Ordering::Acquire) {
        panic!("dma-api not initialized");
    }
    unsafe { OSAL }
}

fn map(addr: NonNull<u8>, size: usize, direction: Direction) -> u64 {
    get_osal().map(addr, size, direction)
}

fn unmap(addr: NonNull<u8>, size: usize) {
    get_osal().unmap(addr, size)
}

fn invalidate(addr: NonNull<u8>, size: usize) {
    get_osal().invalidate(addr, size)
}

fn flush(addr: NonNull<u8>, size: usize) {
    get_osal().flush(addr, size)
}

#[cfg(feature = "alloc")]
fn alloc(dma_mask: u64, layout: core::alloc::Layout) -> *mut u8 {
    unsafe { get_osal().alloc(dma_mask, layout) }
}

#[cfg(feature = "alloc")]
fn dealloc(ptr: *mut u8, layout: core::alloc::Layout) {
    unsafe { get_osal().dealloc(ptr, layout) }
}
