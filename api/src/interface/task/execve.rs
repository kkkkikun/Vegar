use crate::imp::task::sys_execve_impl;
use crate::imp::utils::path::resolve_path;
use crate::ptr::{UserConstPtr, UserInPtr};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use axerrno::LinuxResult;
use axhal::arch::TrapFrame;
use core::ffi::c_char;
use syscall_trace::syscall_trace;

fn get_string_array(array: UserConstPtr<usize>) -> LinuxResult<Vec<String>> {
    let string_ptrs = array.get_as_null_terminated()?;
    let string_ptrs = string_ptrs
        .iter()
        .map(|ptr| UserConstPtr::<c_char>::from(*ptr));
    let strings: Vec<String> = string_ptrs
        .map(|ptr| ptr.get_as_str().map(Into::into))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(strings)
}

#[syscall_trace]
pub fn sys_execve(
    tf: &mut TrapFrame,
    path: UserInPtr<c_char>,
    argv: UserInPtr<usize>,
    envp: UserInPtr<usize>,
) -> LinuxResult<isize> {
    let path = path.get_as_str()?;
    let args = get_string_array(argv.clone())?;
    let envs = get_string_array(envp.clone())?;

    // TODO: enhance it
    if path.ends_with(".sh") {
        const BUSYBOX: &str = "/musl/busybox";
        info!("[execve] shebang detected, calling sh...");
        let mut new_args = vec![BUSYBOX.to_string(), "sh".to_string()];
        new_args.extend(args);
        sys_execve_impl(tf, BUSYBOX.to_string(), new_args, envs)
    } else {
        let abs_path = resolve_path(path)?;
        let mut new_args = vec![abs_path.to_string()];
        new_args.extend(args[1..].iter().cloned());
        sys_execve_impl(tf, abs_path.to_string(), new_args, envs)
    }
}
