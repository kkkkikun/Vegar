use crate::imp::task::resource::{sys_getrlimit_impl, sys_setrlimit_impl};
use crate::ptr::{PtrWrapper, UserInPtr, UserOutPtr};
use axerrno::{LinuxError, LinuxResult};
use core::ffi::c_int;
use starry_core::resource::{ResourceLimit, ResourceLimitType};
use syscall_trace::syscall_trace;

#[syscall_trace]
pub fn sys_prlimit64(
    pid: c_int,
    resource: c_int,
    new_limit: UserInPtr<ResourceLimit>,
    old_limit: UserOutPtr<ResourceLimit>,
) -> LinuxResult<isize> {
    let resource = ResourceLimitType::try_from(resource as u32).map_err(|_| LinuxError::EINVAL)?;
    if !old_limit.is_null() {
        let old_value = sys_getrlimit_impl(&resource, pid as _)?;
        unsafe { old_limit.get()?.write(old_value); }
    }
    if !new_limit.is_null() {
        let new_value = new_limit.get_as_ref()?;
        sys_setrlimit_impl(&resource, new_value, pid as _)?;
    }
    Ok(0)
}

#[syscall_trace]
pub fn sys_setrlimit(
    resource: c_int,
    resource_limit: UserInPtr<ResourceLimit>,
) -> LinuxResult<isize> {
    let resource = ResourceLimitType::try_from(resource as u32).map_err(|_| LinuxError::EINVAL)?;
    sys_setrlimit_impl(&resource, resource_limit.get_as_ref()?, 0)
}

#[syscall_trace]
pub fn sys_getrlimit(
    resource: c_int,
    resource_limit: UserOutPtr<ResourceLimit>,
) -> LinuxResult<isize> {
    let resource = ResourceLimitType::try_from(resource as u32).map_err(|_| LinuxError::EINVAL)?;
    let old_value = sys_getrlimit_impl(&resource, 0)?;
    unsafe{resource_limit.get()?.write(old_value);}
    Ok(0)
}
