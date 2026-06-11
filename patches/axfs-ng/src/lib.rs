//! ArceOS filesystem module.
//!
//! Provides high-level filesystem operations built on top of the VFS layer,
//! including file I/O with page caching, directory traversal, and
//! `std::fs`-like APIs.

#![cfg_attr(all(not(test), not(doc)), no_std)]
#![feature(doc_cfg)]
#![allow(clippy::new_ret_no_self)]

extern crate alloc;

#[macro_use]
extern crate log;

use axdriver::{AxBlockDevice, AxDeviceContainer, prelude::*};

mod fs;

mod highlevel;
pub use highlevel::*;

/// Initializes the filesystem subsystem using the first available block device.
///
/// In evaluation QEMU, two block devices are present: x0 (test disk) and
/// x1 (auxiliary data disk). `take_one()` pops from the back (LIFO) and
/// would give x1. We collect all devices into a temporary Vec instead,
/// then pop from its back to get the first-enumerated device (x0 = test disk).
pub fn init_filesystems(mut block_devs: AxDeviceContainer<AxBlockDevice>) {
    info!("Initialize filesystem subsystem...");
    info!("  found {} block device(s)", block_devs.len());

    let mut all_devs = alloc::vec::Vec::new();
    while let Some(d) = block_devs.take_one() {
        info!("  found {:?}", d.device_name());
        all_devs.push(d);
    }
    let dev = all_devs.pop().expect("No block device found!");
    info!("  using as root: {:?}", dev.device_name());

    let fs = fs::new_default(dev).expect("Failed to initialize filesystem");
    info!("  filesystem type: {:?}", fs.name());

    let mp = axfs_ng_vfs::Mountpoint::new_root(&fs);
    ROOT_FS_CONTEXT.call_once(|| FsContext::new(mp.root_location()));
}
