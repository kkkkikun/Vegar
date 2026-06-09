use alloc::vec::Vec;
use core::{
    alloc::Layout,
    ptr::{slice_from_raw_parts_mut, NonNull},
};

use crate::{flush, map, unmap, Direction};

pub mod r#box;
pub mod pool;
pub mod vec;

#[derive(thiserror::Error, Debug, Clone)]
pub enum DError {
    #[error("DMA mask not match, required {mask:#x}, got {got:#x}")]
    DmaMaskNotMatch { mask: u64, got: u64 },
    #[error("No memory")]
    NoMemory,
    #[error("Layout error")]
    LayoutError,
}

impl From<core::alloc::LayoutError> for DError {
    fn from(_: core::alloc::LayoutError) -> Self {
        DError::LayoutError
    }
}

struct DCommon<T> {
    addr: NonNull<T>,
    bus_addr: u64,
    layout: Layout,
    direction: Direction,
}

unsafe impl<T: Send> Send for DCommon<T> {}

impl<T> DCommon<T> {
    pub fn zeros(dma_mask: u64, layout: Layout, direction: Direction) -> Result<Self, DError> {
        unsafe {
            let mut addr = NonNull::new(crate::alloc(dma_mask, layout)).ok_or(DError::NoMemory)?;
            (*slice_from_raw_parts_mut(addr.as_mut(), layout.size())).fill(0);

            let bus_addr = map(addr, layout.size(), direction);
            if let Err(e) = Self::check_dma_mask(dma_mask, bus_addr) {
                crate::dealloc(addr.as_ptr() as _, layout);
                return Err(e);
            }
            flush(addr, layout.size());
            Ok(Self {
                bus_addr,
                addr: addr.cast(),
                layout,
                direction,
            })
        }
    }

    fn check_dma_mask(dma_mask: u64, bus_addr: u64) -> Result<(), DError> {
        if (bus_addr) & (dma_mask) != (bus_addr) {
            return Err(DError::DmaMaskNotMatch {
                mask: dma_mask,
                got: bus_addr,
            });
        }
        Ok(())
    }

    pub fn from_vec(
        dma_mask: u64,
        mut value: Vec<T>,
        direction: Direction,
    ) -> Result<Self, DError> {
        unsafe {
            let layout = Layout::from_size_align_unchecked(
                value.capacity() * size_of::<T>(),
                align_of::<T>(),
            );

            let addr = NonNull::new(value.as_mut_ptr()).unwrap();

            let bus_addr = map(addr.cast(), layout.size(), direction);
            Self::check_dma_mask(dma_mask, bus_addr)?;

            core::mem::forget(value);

            flush(addr.cast(), layout.size());
            Ok(Self {
                bus_addr,
                addr: addr.cast(),
                layout,
                direction,
            })
        }
    }

    pub fn prepare_read(&self, ptr: NonNull<u8>, size: usize) {
        self.direction.prepare_read(ptr, size);
    }

    pub fn confirm_write(&self, ptr: NonNull<u8>, size: usize) {
        self.direction.confirm_write(ptr, size);
    }

    pub fn confirm_write_all(&self) {
        self.direction
            .confirm_write(self.addr.cast(), self.layout.size());
    }
}

impl<T> Drop for DCommon<T> {
    fn drop(&mut self) {
        if self.layout.size() > 0 {
            unmap(self.addr.cast(), self.layout.size());

            crate::dealloc(self.addr.as_ptr() as _, self.layout);
        }
    }
}
