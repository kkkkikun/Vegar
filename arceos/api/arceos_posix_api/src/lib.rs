//! POSIX-compatible APIs for [ArceOS] modules
//!
//! [ArceOS]: https://github.com/arceos-org/arceos

#![cfg_attr(all(not(test), not(doc)), no_std)]
#![feature(doc_cfg)]
#![feature(doc_auto_cfg)]
#![allow(clippy::missing_safety_doc)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[macro_use]
extern crate axlog;
extern crate axruntime;

#[macro_use]
mod utils;

mod imp;
pub use utils::char_ptr_to_str;

/// Platform-specific constants and parameters.
pub mod config {
    pub use axconfig::*;
}

/// POSIX C types.
#[rustfmt::skip]
#[path = "./ctypes_gen.rs"]
#[allow(dead_code, non_snake_case, non_camel_case_types, non_upper_case_globals, clippy::upper_case_acronyms, missing_docs)]
pub mod ctypes;

pub use imp::io::*;
#[cfg(feature = "fs")]
pub use imp::path_link::{AT_FDCWD, FilePath, HARDLINK_MANAGER, handle_file_path};
pub use imp::resources::{sys_getrlimit, sys_setrlimit};
pub use imp::sys::sys_sysconf;
pub use imp::task::{sys_exit, sys_getpid, sys_sched_yield};
pub use imp::time::{sys_clock_gettime, sys_get_time_of_day, sys_nanosleep};

#[cfg(feature = "fd")]
pub use imp::fd_ops::*;
#[cfg(feature = "fs")]
pub use imp::fs::{Directory, File, sys_open, sys_openat};
#[cfg(feature = "select")]
pub use imp::io_mpx::sys_select;
#[cfg(feature = "epoll")]
pub use imp::io_mpx::{sys_epoll_create, sys_epoll_ctl, sys_epoll_wait};
#[cfg(feature = "net")]
pub use imp::net::*;
#[cfg(feature = "pipe")]
pub use imp::pipe::*;
#[cfg(feature = "multitask")]
pub use imp::pthread::mutex::{
    sys_pthread_mutex_init, sys_pthread_mutex_lock, sys_pthread_mutex_unlock,
};
#[cfg(feature = "multitask")]
pub use imp::pthread::{sys_pthread_create, sys_pthread_exit, sys_pthread_join, sys_pthread_self};
