//! POSIX-like multitasking and process management.
//! Includes a strict two-level hierarchy of processes (session and process group).
//! See https://man7.org/linux/man-pages/man7/credentials.7.html for more details.
//! Session contain ProcessGroup, ProcessGroup contain Process, Process contain Thread.
//! Process has child processes.
#![no_std]

extern crate alloc;
pub mod process;
pub mod process_group;
pub mod session;
pub mod thread;

/// Type alias for session ID, process group ID, process ID, and thread ID.
/// Linux uses `int` for these IDs, which is typically 32 bits.
pub type Pid = u32;
