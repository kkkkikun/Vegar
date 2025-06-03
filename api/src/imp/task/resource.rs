use axerrno::{LinuxError, LinuxResult};
use starry_core::process::*;
use starry_core::resource::{ResourceLimit, ResourceLimitType};
use starry_core::task::current_process_data;
use undefined_process::Pid;

pub fn sys_setrlimit_impl(
    resource: &ResourceLimitType,
    limit: &ResourceLimit,
    pid: Pid,
) -> LinuxResult<isize> {
    let process_data = if pid == 0 {
        current_process_data()
    } else {
        get_process_data(pid as _).ok_or(LinuxError::ESRCH)?
    };

    let mut limits = process_data.resource_limits.lock();
    let old_limit = limits.get(resource);
    if limit.hard > old_limit.hard {
        return Err(LinuxError::EPERM);
    }
    if !limits.set(resource, limit.clone()) {
        return Err(LinuxError::EINVAL); // soft > hard
    }
    Ok(0)
}

pub fn sys_getrlimit_impl(resource: &ResourceLimitType, pid: Pid) -> LinuxResult<ResourceLimit> {
    let process_data = if pid == 0 {
        current_process_data()
    } else {
        get_process_data(pid as _).ok_or(LinuxError::ESRCH)?
    };
    Ok(process_data.resource_limits.lock().get(resource))
}
