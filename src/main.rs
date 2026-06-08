#![no_std]
#![no_main]
extern crate alloc;
use alloc::{borrow::ToOwned, vec::Vec};
#[cfg(feature = "ltp-only")]
pub const CMDLINE: &[&str] = &["/musl/busybox", "sh", "-c", include_str!("init_ltp.sh")];

#[cfg(feature = "custom")]
pub const CMDLINE: &[&str] = &["/musl/busybox", "sh", "-c", include_str!("init_custom.sh")];

#[cfg(not(any(feature = "ltp-only", feature = "custom")))]
pub const CMDLINE: &[&str] = &["/musl/busybox", "sh", "-c", include_str!("init_oscomp.sh")];
#[unsafe(no_mangle)]
fn main() {
    let args = CMDLINE.iter().copied().map(str::to_owned).collect::<Vec<_>>();
    let envs = [];
    starry_kernel::entry::init(&args, &envs);
}
