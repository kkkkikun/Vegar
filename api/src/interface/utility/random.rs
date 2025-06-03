use crate::ptr::UserOutPtr;
use axerrno::LinuxResult;
use axsync::Mutex;
use core::ffi::c_uint;
use lazy_static::lazy_static;
use rand_mt::Mt64;
use syscall_trace::syscall_trace;

lazy_static! {
    /// A globally accessible random number generator.
    pub static ref RANDOM_GENERATOR: Mutex<Mt64> = {
        let seed = axhal::time::monotonic_time_nanos();
        Mutex::new(Mt64::new(seed))
    };
}

#[syscall_trace]
pub fn sys_getrandom(buf: UserOutPtr<u8>, len: usize, _flags: c_uint) -> LinuxResult<isize> {
    let buf = buf.get_as_mut_slice(len)?;
    let mut rand = RANDOM_GENERATOR.lock();
    rand.fill_bytes(buf);
    Ok(len as isize)
}
