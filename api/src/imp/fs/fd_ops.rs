use core::ffi::c_int;

use arceos_posix_api as api;
use arceos_posix_api::{FD_TABLE, add_file_like, close_file_like, ctypes, get_file_like};
use axerrno::{LinuxError, LinuxResult};
use starry_core::resource::ResourceLimitType;
use starry_core::task::current_process_data;

pub fn sys_dup(old_fd: c_int) -> LinuxResult<isize> {
    let limit = current_process_data()
        .resource_limits
        .lock()
        .get_soft(&ResourceLimitType::NOFILE);
    if FD_TABLE.read().count() >= limit as usize {
        // 成功时返回新文件描述符，失败时返回 -1 并设置 errno 指示具体错误。
        return Err(LinuxError::EMFILE);
    }

    let f = get_file_like(old_fd)?;
    let new_fd = add_file_like(f)?;
    Ok(new_fd as _)
}

pub fn sys_dup2(old_fd: c_int, new_fd: c_int) -> LinuxResult<isize> {
    debug!("sys_dup2 <= old_fd: {}, new_fd: {}", old_fd, new_fd);
    if old_fd == new_fd {
        let r = sys_fcntl(old_fd, ctypes::F_GETFD as _, 0)?;
        return if r >= 0 { Ok(old_fd as _) } else { Ok(r) };
    }
    let limit = current_process_data()
        .resource_limits
        .lock()
        .get_soft(&ResourceLimitType::NOFILE);
    if new_fd as u64 >= limit {
        return Err(LinuxError::EBADF);
    }

    // check if the old fd is open
    let f = get_file_like(old_fd)?;
    // close the new_fd if it is already opened
    // ignore any error during the close
    close_file_like(new_fd).unwrap_or(());
    FD_TABLE
        .write()
        .add_at(new_fd as usize, f)
        .map_err(|_| LinuxError::EMFILE)?;

    Ok(new_fd as _)
}

pub fn sys_dup3(old_fd: c_int, new_fd: c_int) -> LinuxResult<isize> {
    debug!("sys_dup3 <= old_fd: {}, new_fd: {}", old_fd, new_fd);
    sys_dup2(old_fd, new_fd)
}

pub fn sys_close(fd: c_int) -> LinuxResult<isize> {
    Ok(api::sys_close(fd) as _)
}

pub fn sys_fcntl(fd: c_int, cmd: c_int, arg: usize) -> LinuxResult<isize> {
    Ok(api::sys_fcntl(fd, cmd, arg) as _)
}
