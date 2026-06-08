use core::alloc::Layout;

use crate::{dma::alloc::DError, Direction};

use super::DCommon;

pub struct DBox<T> {
    inner: DCommon<T>,
}

impl<T> DBox<T> {
    const SIZE: usize = core::mem::size_of::<T>();

    pub fn zero_with_align(
        dma_mask: u64,
        direction: Direction,
        align: usize,
    ) -> Result<Self, super::DError> {
        let layout = Layout::from_size_align(Self::SIZE, align)?;

        Ok(Self {
            inner: DCommon::zeros(dma_mask, layout, direction)?,
        })
    }

    pub fn zero(dma_mask: u64, direction: Direction) -> Result<Self, DError> {
        let layout = Layout::new::<T>();
        Ok(Self {
            inner: DCommon::zeros(dma_mask, layout, direction)?,
        })
    }
    pub fn bus_addr(&self) -> u64 {
        self.inner.bus_addr
    }

    pub fn read(&self) -> T {
        unsafe {
            let ptr = self.inner.addr;

            self.inner.prepare_read(ptr.cast(), Self::SIZE);

            ptr.read_volatile()
        }
    }

    pub fn write(&mut self, value: T) {
        unsafe {
            let ptr = self.inner.addr;

            ptr.write_volatile(value);

            self.inner.confirm_write(ptr.cast(), Self::SIZE);
        }
    }

    pub fn modify(&mut self, f: impl FnOnce(&mut T)) {
        unsafe {
            let mut ptr = self.inner.addr;

            self.inner.prepare_read(ptr.cast(), Self::SIZE);

            f(ptr.as_mut());

            self.inner.confirm_write(ptr.cast(), Self::SIZE);
        }
    }
}
