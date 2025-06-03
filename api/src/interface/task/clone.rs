//! sys_clone:
//! # Arguments
//! - `addr_parent_tid`: Where to store child TID in parent's memory.
//! - `addr_child_tid`: Where to store child TID in child's memory.
//! - `new_sp`: New stack pointer, pointer to the lowest byte of stack.
//! - `tls`: Thread Local Storage descriptor. The interpretation of tls and the resulting effect is
//!          architecture dependent. For example, on architectures with a dedicated TLS register,
//!          it is the new value of that register.
//!
//! # Archs
//! Different arch has different definition of `sys_clone`,
//! see Linux source code `kernel/fork.c`
//! - riscv: `CONFIG_CLONE_BACKWARDS`
//! - aarch: `CONFIG_CLONE_BACKWARDS`
//! - x86_32: `CONFIG_CLONE_BACKWARDS`
//! - x86_64: `NONE`
//! - loongarch: `NONE`
use crate::imp::task::*;
use crate::ptr::{PtrWrapper, UserOutPtr};
use axerrno::{LinuxError, LinuxResult};
use axsignal::Signo;
use core::ffi::{c_int, c_ulong};
use linux_raw_sys::general::CSIGNAL;
use syscall_trace::syscall_trace;
use undefined_process::Pid;

// definition for `CONFIG_CLONE_BACKWARDS`
#[cfg(any(target_arch = "x86_64", target_arch = "loongarch64"))]
#[syscall_trace]
pub fn sys_clone(
    clone_flags: c_ulong,
    new_sp: c_ulong,
    addr_parent_tid: UserOutPtr<c_int>,
    addr_child_tid: UserOutPtr<c_int>,
    tls: c_ulong,
) -> LinuxResult<isize> {
    sys_clone_(
        clone_flags,
        new_sp,
        addr_parent_tid.clone(),
        addr_child_tid.clone(),
        tls,
    )
}

// definition for `NONE`
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
#[syscall_trace]
pub fn sys_clone(
    clone_flags: c_ulong,
    new_sp: c_ulong,
    addr_parent_tid: UserOutPtr<c_int>,
    tls: c_ulong,
    addr_child_tid: UserOutPtr<c_int>,
) -> LinuxResult<isize> {
    sys_clone_(
        clone_flags,
        new_sp,
        addr_parent_tid.clone(),
        addr_child_tid.clone(),
        tls,
    )
}

fn sys_clone_(
    flags: c_ulong,
    new_sp: c_ulong,
    addr_parent_tid: UserOutPtr<c_int>,
    addr_child_tid: UserOutPtr<c_int>,
    tls: c_ulong,
) -> LinuxResult<isize> {
    // get flags
    let flags = flags as u32; // lower 32 bits of clone_flags
    let exit_signal = flags & CSIGNAL;
    let clone_flags = flags & !CSIGNAL;
    let exit_signal = Signo::from_repr(exit_signal as u8);
    let clone_flags = CloneFlags::from_bits_truncate(clone_flags);

    // param check
    // If CLONE_THREAD or CLONE_PARENT was specified in the flags,
    // a signal must not be specified in exit_signal.
    if !exit_signal.is_none() && clone_flags.contains(CloneFlags::THREAD | CloneFlags::PARENT) {
        return Err(LinuxError::EINVAL);
    }
    // Since Linux 2.6.0, the flags mask must also include CLONE_VM if CLONE_SIGHAND is specified.
    if clone_flags.contains(CloneFlags::SIGHAND) && !clone_flags.contains(CloneFlags::VM) {
        return Err(LinuxError::EINVAL);
    }
    // Since Linux 2.5.35, the flags mask must also include CLONE_SIGHAND if CLONE_THREAD is specified.
    // And note that, since Linux 2.6.0, CLONE_SIGHAND also requires CLONE_VM to be included.
    if clone_flags.contains(CloneFlags::THREAD)
        && !clone_flags.contains(CloneFlags::VM | CloneFlags::SIGHAND)
    {
        return Err(LinuxError::EINVAL);
    }
    // TODO: signals
    let result = sys_clone_impl(
        clone_flags,
        new_sp as _,
        tls as _,
        addr_child_tid.address().into(),
        exit_signal,
    );

    if let Ok(tid) = result {
        let tid = tid as Pid;
        if clone_flags.contains(CloneFlags::PARENT_SETTID) {
            // Store the child thread ID at the location pointed to by `addr_parent_tid`
            unsafe {
                addr_parent_tid.get()?.write(tid as _);
            }
        }
        // CHILD_SETTID is done in `sys_clone_impl`
    }

    result
}

#[syscall_trace]
pub fn sys_fork() -> LinuxResult<isize> {
    // fork is a special case of clone
    // TODO: exit_signal = SIGCHLD
    sys_clone_impl(CloneFlags::empty(), 0, 0, 0, None)
}
