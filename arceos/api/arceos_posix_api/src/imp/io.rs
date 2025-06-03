#[cfg(feature = "fd")]
use crate::imp::fd_ops::get_file_like;
use crate::{File, ctypes};
use axerrno::{LinuxError, LinuxResult};
use axio::SeekFrom;
#[cfg(not(feature = "fd"))]
use axio::prelude::*;
use core::ffi::{c_int, c_void};

/// Read data from the file indicated by `fd`.
///
/// Return the read size if success.
pub fn sys_read(fd: c_int, buf: *mut c_void, count: usize) -> ctypes::ssize_t {
    debug!("sys_read <= {} {:#x} {}", fd, buf as usize, count);
    syscall_body!(sys_read, {
        if buf.is_null() {
            return Err(LinuxError::EFAULT);
        }
        let dst = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, count) };
        #[cfg(feature = "fd")]
        {
            Ok(get_file_like(fd)?.read(dst)? as ctypes::ssize_t)
        }
        #[cfg(not(feature = "fd"))]
        match fd {
            0 => Ok(super::stdio::stdin().read(dst)? as ctypes::ssize_t),
            1 | 2 => Err(LinuxError::EPERM),
            _ => Err(LinuxError::EBADF),
        }
    })
}

fn write_impl(fd: c_int, buf: *const c_void, count: usize) -> LinuxResult<ctypes::ssize_t> {
    if buf.is_null() {
        return Err(LinuxError::EFAULT);
    }
    let src = unsafe { core::slice::from_raw_parts(buf as *const u8, count) };
    #[cfg(feature = "fd")]
    {
        Ok(get_file_like(fd)?.write(src)? as ctypes::ssize_t)
    }
    #[cfg(not(feature = "fd"))]
    match fd {
        0 => Err(LinuxError::EPERM),
        1 | 2 => Ok(super::stdio::stdout().write(src)? as ctypes::ssize_t),
        _ => Err(LinuxError::EBADF),
    }
}

/// Write data to the file indicated by `fd`.
///
/// Return the written size if success.
pub fn sys_write(fd: c_int, buf: *const c_void, count: usize) -> ctypes::ssize_t {
    debug!("sys_write <= {} {:#x} {}", fd, buf as usize, count);
    syscall_body!(sys_write, write_impl(fd, buf, count))
}

/// Write a vector.
pub unsafe fn sys_writev(fd: c_int, iov: *const ctypes::iovec, iocnt: c_int) -> ctypes::ssize_t {
    debug!("sys_writev <= fd: {}", fd);
    syscall_body!(sys_writev, {
        if !(0..=1024).contains(&iocnt) {
            return Err(LinuxError::EINVAL);
        }

        let iovs = unsafe { core::slice::from_raw_parts(iov, iocnt as usize) };
        let mut ret = 0;
        for iov in iovs.iter() {
            // TODO: if the `unwrap_or(0)` is correct?
            let result = write_impl(fd, iov.iov_base, iov.iov_len).unwrap_or(0);
            ret += result;

            if result < iov.iov_len as isize {
                break;
            }
        }

        Ok(ret)
    })
}

/// Read a vector
pub unsafe fn sys_readv(fd: c_int, iov: *const ctypes::iovec, iocnt: c_int) -> ctypes::ssize_t {
    debug!("sys_readv <= fd: {}", fd);
    syscall_body!(sys_readv, {
        if !(0..=1024).contains(&iocnt) {
            return Err(LinuxError::EINVAL);
        }

        let iovs = unsafe { core::slice::from_raw_parts(iov, iocnt as usize) };
        let mut ret = 0;
        for iov in iovs.iter() {
            let result = sys_read(fd, iov.iov_base, iov.iov_len as usize);
            ret += result;

            if result < iov.iov_len as isize {
                break;
            }
        }

        Ok(ret)
    })
}

/// sendfile: transfer data between file descriptors
///
/// The `sendfile()` system call copies data between one file descriptor (`in_fd`)
/// and another (`out_fd`). This operation occurs entirely within the kernel,
/// avoiding unnecessary data copies to user space, making it significantly
/// faster than `read()` + `write()` for large transfers.
///
/// # Arguments
/// - `out_fd`: Output file descriptor (must be opened for writing).
///   - Supported types: Regular files, sockets (historical limitation), pipes (Linux 5.12+).
/// - `in_fd`: Input file descriptor (must be opened for reading).
///   - Supported types: Regular files, block devices (mmap-able). See restrictions below.
/// - `offset`:
///   - If non-null: Specifies the starting offset in `in_fd` to read from.
///     The kernel updates the pointed value to the next byte after the last read.
///   - If null: Reads from `in_fd`'s current file offset and updates it automatically.
/// - `count`: Maximum number of bytes to transfer (actual transferred bytes may be less).
///
/// # Returns
/// - On success: Returns the number of bytes transferred (≥0).
///   Caller must check and retry if less than `count`.
/// - On error: Returns `-1` with `errno` set to indicate the error.
///
pub fn sys_sendfile(
    out_fd: c_int,
    in_fd: c_int,
    offset: *mut ctypes::off_t,
    count: usize,
) -> ctypes::ssize_t {
    debug!(
        "[sys_sendfile] out_fd={}, in_fd={}, offset={:#x}, count={}",
        out_fd, in_fd, offset as usize, count
    );
    syscall_body!(sys_sendfile, {
        #[cfg(feature = "fd")]
        {
            let in_file = get_file_like(in_fd)?;
            if !in_file.poll()?.readable {
                return Err(LinuxError::EBADF);
            }
            let out_file = get_file_like(out_fd)?;
            if !out_file.poll()?.writable {
                return Err(LinuxError::EBADF);
            }

            let mut origin_offset = 0;
            if !offset.is_null() {
                match in_file.clone().into_any().downcast_ref::<File>() {
                    Some(file) => {
                        let file = file.inner();
                        // save origin offset
                        origin_offset = file.lock().seek(SeekFrom::Current(0))?;
                        // seek to the offset
                        unsafe {
                            file.lock().seek(SeekFrom::Start(*offset as _))?;
                        }
                    }
                    None => {
                        // The in_file must be seekable
                        return Err(LinuxError::ESPIPE);
                    }
                }
            };

            // transfer file data
            let mut remaining = count;
            let mut buffer = [0u8; 4096];
            while remaining != 0 {
                let bytes_read = in_file.read(&mut buffer)?;
                if bytes_read == 0 {
                    // maybe the low level api will return 0 at EOF?
                    break; // EOF
                }
                let bytes_write = out_file.write(&buffer[0..bytes_read])?;
                if bytes_write < bytes_read {
                    remaining -= bytes_write;
                    // break if partial write encountered
                    break;
                } else {
                    remaining -= bytes_read;
                }
            }

            if !offset.is_null() {
                match in_file.into_any().downcast_ref::<File>() {
                    Some(file) => {
                        let file = file.inner();
                        // save current offset
                        unsafe {
                            *offset = file.lock().seek(SeekFrom::Current(0))? as _;
                        }
                        // restore the origin offset
                        file.lock().seek(SeekFrom::Start(origin_offset))?;
                    }
                    None => {
                        // The in_file must be seekable
                        return Err(LinuxError::ESPIPE);
                    }
                }
            }

            Ok((count - remaining) as ctypes::ssize_t)
        }
        #[cfg(not(feature = "fd"))]
        {
            warn!("[sys_sendfile] sendfile is not supported on this platform");
        }
    })
}
