#![cfg_attr(not(test), no_std)]

#[cfg(target_arch = "aarch64")]
pub mod asm;
#[cfg(target_arch = "aarch64")]
pub mod cache;
#[cfg(target_arch = "aarch64")]
pub mod registers {
    pub use aarch64_cpu::registers::*;
}

pub mod structures;

#[cfg(test)]
mod test {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
