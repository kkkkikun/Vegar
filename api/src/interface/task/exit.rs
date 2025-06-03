use crate::imp::task::sys_exit_impl;
use axerrno::LinuxResult;
use core::ffi::c_int;
use syscall_trace::syscall_trace;

#[syscall_trace]
pub fn sys_exit(status: c_int) -> LinuxResult<isize> {
    sys_exit_impl(status, false)
}

#[syscall_trace]
pub fn sys_exit_group(status: c_int) -> LinuxResult<isize> {
    sys_exit_impl(status, true)
}
