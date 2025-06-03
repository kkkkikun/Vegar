use crate::imp::fs::{sys_pread_impl, sys_pwrite_impl, sys_truncate_impl};
use crate::ptr::{UserInPtr, UserOutPtr};
use arceos_posix_api::{File, get_file_like};
use axerrno::{LinuxError, LinuxResult};
use axfs::fops;
use axfs::fops::OpenOptions;
use core::ffi::{c_char, c_int, c_long};
use syscall_trace::syscall_trace;

#[syscall_trace]
pub fn sys_truncate(path: UserInPtr<c_char>, length: c_long) -> LinuxResult<isize> {
    // get params
    let path = path.get_as_str()?;

    // open file
    let mut options = OpenOptions::new();
    options.write(true);

    let file = fops::File::open(path, &options)?;
    sys_truncate_impl(&file, length as _)
}

#[syscall_trace]
pub fn sys_ftruncate(fd: c_int, length: c_long) -> LinuxResult<isize> {
    let file_like = get_file_like(fd)?.into_any();
    let api_file = file_like.downcast_ref::<File>().ok_or(LinuxError::EINVAL)?;
    sys_truncate_impl(&api_file.inner().lock(), length as _)
}

#[syscall_trace]
pub fn sys_pread64(
    fd: c_int,
    buf: UserOutPtr<u8>,
    count: usize,
    offset: usize,
) -> LinuxResult<isize> {
    let buf = buf.get_as_mut_slice(count)?;
    sys_pread_impl(fd, buf, offset as _)
}

#[syscall_trace]
pub fn sys_pwrite64(
    fd: c_int,
    buf: UserInPtr<u8>,
    count: usize,
    offset: usize,
) -> LinuxResult<isize> {
    let buf = buf.get_as_slice(count)?;
    sys_pwrite_impl(fd, buf, offset as _)
}
