use core::{
    marker::PhantomData,
    mem::{size_of, size_of_val},
    ops::Index,
    ptr::NonNull,
};

use crate::{flush, map, unmap, Direction};

#[repr(transparent)]
pub struct DSlice<'a, T> {
    inner: DSliceCommon<'a, T>,
}

impl<'a, T> DSlice<'a, T> {
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn bus_addr(&self) -> u64 {
        self.inner.bus_addr
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn from(value: &'a [T], direction: Direction) -> Self {
        Self {
            inner: DSliceCommon::new(value, direction),
        }
    }

    pub fn prepare_read_all(&self) {
        self.inner.prepare_read_all();
    }

    pub fn confirm_write_all(&self) {
        self.inner.confirm_write_all();
    }
}

impl<T> Index<usize> for DSlice<'_, T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.inner.index(index)
    }
}

impl<T> AsRef<[T]> for DSlice<'_, T> {
    fn as_ref(&self) -> &[T] {
        self.inner.as_ref()
    }
}

#[repr(transparent)]
pub struct DSliceMut<'a, T> {
    inner: DSliceCommon<'a, T>,
}

impl<'a, T> DSliceMut<'a, T> {
    pub fn from(value: &'a mut [T], direction: Direction) -> Self {
        Self {
            inner: DSliceCommon::new(value, direction),
        }
    }

    pub fn bus_addr(&self) -> u64 {
        self.inner.bus_addr
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn set(&self, index: usize, value: T) {
        assert!(index < self.len());

        unsafe {
            let ptr = self.inner.addr.add(index);

            ptr.write_volatile(value);

            self.inner
                .direction
                .confirm_write(ptr.cast(), size_of::<T>());
        }
    }

    pub fn prepare_read_all(&self) {
        self.inner.prepare_read_all();
    }

    pub fn confirm_write_all(&self) {
        self.inner.confirm_write_all();
    }
}

impl<T> Index<usize> for DSliceMut<'_, T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.inner.index(index)
    }
}

impl<T> AsRef<[T]> for DSliceMut<'_, T> {
    fn as_ref(&self) -> &[T] {
        self.inner.as_ref()
    }
}

struct DSliceCommon<'a, T> {
    addr: NonNull<T>,
    size: usize,
    bus_addr: u64,
    direction: Direction,
    _marker: PhantomData<&'a T>,
}

impl<'a, T> DSliceCommon<'a, T> {
    fn new(s: &'a [T], direction: Direction) -> Self {
        let size = size_of_val(s);
        let ptr = unsafe { NonNull::new_unchecked(s.as_ptr() as usize as *mut T) };
        let bus_addr = map(ptr.cast(), size, direction);

        flush(ptr.cast(), size);

        Self {
            addr: ptr,
            size,
            bus_addr,
            direction,
            _marker: PhantomData,
        }
    }

    fn len(&self) -> usize {
        self.size / size_of::<T>()
    }

    fn index(&self, index: usize) -> &T {
        assert!(index < self.len());

        let ptr = unsafe { self.addr.add(index) };

        self.direction.prepare_read(ptr.cast(), size_of::<T>());

        unsafe { ptr.as_ref() }
    }

    fn prepare_read_all(&self) {
        self.direction
            .prepare_read(self.addr.cast(), self.size * size_of::<T>());
    }

    fn confirm_write_all(&self) {
        self.direction
            .confirm_write(self.addr.cast(), self.size * size_of::<T>());
    }
}

impl<T> Drop for DSliceCommon<'_, T> {
    fn drop(&mut self) {
        unmap(self.addr.cast(), self.size);
    }
}

impl<T> AsRef<[T]> for DSliceCommon<'_, T> {
    fn as_ref(&self) -> &[T] {
        self.prepare_read_all();
        unsafe { core::slice::from_raw_parts_mut(self.addr.as_ptr(), self.len()) }
    }
}
