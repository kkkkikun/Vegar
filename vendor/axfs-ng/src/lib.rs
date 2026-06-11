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
pub fn init_filesystems(mut block_devs: AxDeviceContainer<AxBlockDevice>) {
    info!("Initialize filesystem subsystem...");

    // Pop all devices and keep the first-enumerated one (x0 = test disk).
    // take_one() uses SmallVec::pop() (LIFO), which would return x1 (data disk)
    // in dual-disk evaluation environments.
    let mut all_devs = alloc::vec::Vec::new();
    while let Some(d) = block_devs.take_one() {
        all_devs.push(d);
    }
    let dev = all_devs.pop().expect("No block device found!");
    info!("  use block device 0: {:?}", dev.device_name());

    let fs = fs::new_default(dev).expect("Failed to initialize filesystem");
    info!("  filesystem type: {:?}", fs.name());

    let mp = axfs_ng_vfs::Mountpoint::new_root(&fs);
    ROOT_FS_CONTEXT.call_once(|| FsContext::new(mp.root_location()));
}
