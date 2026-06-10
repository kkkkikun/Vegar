use core::ptr::NonNull;

pub fn flush(_addr: NonNull<u8>, _size: usize) {}

pub fn invalidate(_addr: NonNull<u8>, _size: usize) {}
