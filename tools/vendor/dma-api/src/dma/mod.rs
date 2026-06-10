use crate::{flush, invalidate, Direction};
use core::ptr::NonNull;

#[cfg(feature = "alloc")]
pub mod alloc;
pub mod slice;

impl Direction {
    pub fn prepare_read(self, ptr: NonNull<u8>, size: usize) {
        if matches!(self, Direction::FromDevice | Direction::Bidirectional) {
            invalidate(ptr, size);
        }
    }
    pub fn confirm_write(self, ptr: NonNull<u8>, size: usize) {
        if matches!(self, Direction::ToDevice | Direction::Bidirectional) {
            flush(ptr, size)
        }
    }
}
