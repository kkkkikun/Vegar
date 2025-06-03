use crate::ptr::{PtrWrapper, UserConstPtr, UserPtr};
use arceos_posix_api::ctypes::off_t;
use arceos_posix_api::{self as api, ctypes::mode_t, get_file_like};
use axerrno::{LinuxError, LinuxResult};
use axfs::fops::File;
use axio::SeekFrom;
use core::ffi::{c_char, c_int, c_void};

pub fn sys_read(fd: i32, buf: UserPtr<c_void>, count: usize) -> LinuxResult<isize> {
    let buf = buf.get_as_bytes(count)?;
    Ok(api::sys_read(fd, buf, count))
}

pub fn sys_write(fd: i32, buf: UserConstPtr<c_void>, count: usize) -> LinuxResult<isize> {
    let buf = buf.get_as_bytes(count)?;
    Ok(api::sys_write(fd, buf, count))
}

pub fn sys_writev(
    fd: i32,
    iov: UserConstPtr<api::ctypes::iovec>,
    iocnt: i32,
) -> LinuxResult<isize> {
    let iov = iov.get_as_bytes(iocnt as _)?;
    unsafe { Ok(api::sys_writev(fd, iov, iocnt)) }
}

pub fn sys_readv(fd: i32, iov: UserPtr<api::ctypes::iovec>, iocnt: i32) -> LinuxResult<isize> {
    let iov = iov.get_as_bytes(iocnt as _)?;
    unsafe { Ok(api::sys_readv(fd, iov, iocnt)) }
}

pub fn sys_openat(
    dirfd: i32,
    path: UserConstPtr<c_char>,
    flags: i32,
    modes: mode_t,
) -> LinuxResult<isize> {
    let path = path.get_as_null_terminated()?;
    Ok(api::sys_openat(dirfd, path.as_ptr(), flags, modes) as _)
}

pub fn sys_open(path: UserConstPtr<c_char>, flags: i32, modes: mode_t) -> LinuxResult<isize> {
    use arceos_posix_api::AT_FDCWD;
    sys_openat(AT_FDCWD as _, path, flags, modes)
}

pub fn sys_lseek(fd: c_int, offset: isize, whence: c_int) -> LinuxResult<isize> {
    let pos = match whence {
        0 => SeekFrom::Start(offset as _),
        1 => SeekFrom::Current(offset as _),
        2 => SeekFrom::End(offset as _),
        _ => return Err(LinuxError::EINVAL),
    };
    let file_like = get_file_like(fd)?.into_any();
    if let Some(file) = file_like.downcast_ref::<api::File>() {
        let offset = file.inner().lock().seek(pos)?;
        Ok(offset as _)
    } else {
        // For pipes, sockets, FIFOs, they are not seekable, so we return an error.
        Err(LinuxError::ESPIPE)
    }
}

pub fn sys_sendfile(
    out_fd: i32,
    in_fd: i32,
    offset: UserPtr<off_t>,
    count: usize,
) -> LinuxResult<isize> {
    let offset = offset.nullable(UserPtr::get)?;
    Ok(api::sys_sendfile(
        out_fd,
        in_fd,
        offset.unwrap_or(core::ptr::null_mut()),
        count,
    ) as _)
}

pub fn sys_truncate_impl(file: &File, length: isize) -> LinuxResult<isize> {
    // set size to length
    file.truncate(length as u64)
        .map_err(|_| axerrno::LinuxError::EIO)?;
    Ok(0)
}

pub fn sys_pwrite_impl(fd: i32, buf: &[u8], offset: isize) -> LinuxResult<isize> {
    let file = api::File::from_fd(fd)?;
    let file = file.inner();
    let write_len = file.lock().write_at(offset as _, buf)?;
    Ok(write_len as _)
}

pub fn sys_pread_impl(fd: i32, buf: &mut [u8], offset: isize) -> LinuxResult<isize> {
    let file = api::File::from_fd(fd)?;
    let file = file.inner();
    let read_len = file.lock().read_at(offset as _, buf)?;
    Ok(read_len as _)
}
