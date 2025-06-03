use axerrno::LinuxResult;
use syscall_trace::syscall_trace;

#[syscall_trace]
pub fn sys_getgid() -> LinuxResult<isize> {
    // TODO: Implement the actual syscall logic
    Ok(1000)
}

#[syscall_trace]
pub fn sys_getegid() -> LinuxResult<isize> {
    // TODO: Implement the actual syscall logic
    Ok(1000)
}

#[syscall_trace]
pub fn sys_getuid() -> LinuxResult<isize> {
    // TODO: Implement the actual syscall logic
    Ok(1000)
}

#[syscall_trace]
pub fn sys_geteuid() -> LinuxResult<isize> {
    // TODO: Implement the actual syscall logic
    Ok(1000)
}
