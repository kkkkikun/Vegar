use crate::Osal;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "aarch64")] {
        #[path = "aarch64.rs"]
        pub mod arch;
    } else{
        #[path = "nop.rs"]
        pub mod arch;
    }
}

pub struct NopOsal;

#[allow(unused_variables)]
impl Osal for NopOsal {
    fn map(&self, addr: core::ptr::NonNull<u8>, size: usize, direction: crate::Direction) -> u64 {
        unimplemented!()
    }

    fn unmap(&self, addr: core::ptr::NonNull<u8>, size: usize) {
        unimplemented!()
    }

    fn flush(&self, addr: core::ptr::NonNull<u8>, size: usize) {
        unimplemented!()
    }

    fn invalidate(&self, addr: core::ptr::NonNull<u8>, size: usize) {
        unimplemented!()
    }
}
