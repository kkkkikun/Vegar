use axerrno::{LinuxError, LinuxResult};
use axhal::{
    arch::TrapFrame,
    trap::{SYSCALL, register_trap_handler},
};
use starry_api::imp::fs::*;
use starry_api::imp::mm::*;
use starry_api::imp::net::socket::*;
use starry_api::imp::sys::*;
use starry_api::imp::task::signal::*;
use starry_api::imp::task::*;
use starry_api::imp::utils::*;
use starry_api::interface::fs::io::*;
use starry_api::interface::fs::path::*;
use starry_api::interface::fs::poll::*;
use starry_api::interface::fs::*;
use starry_api::interface::mm::shm::*;
use starry_api::interface::task::resource::*;
use starry_api::interface::task::*;
use starry_api::interface::user::identity::*;
use starry_api::interface::utility::random::*;
use starry_core::task::{time_stat_from_kernel_to_user, time_stat_from_user_to_kernel};
use syscalls::Sysno;

#[register_trap_handler(SYSCALL)]
fn handle_syscall(tf: &mut TrapFrame, syscall_num: usize) -> isize {
    info!("[syscall] <{:?}> begin", Sysno::from(syscall_num as u32));
    time_stat_from_user_to_kernel();
    let result: LinuxResult<isize> = match Sysno::from(syscall_num as u32) {
        Sysno::read => sys_read(tf.arg0() as _, tf.arg1().into(), tf.arg2() as _),
        Sysno::write => sys_write(tf.arg0() as _, tf.arg1().into(), tf.arg2() as _),
        Sysno::mmap => sys_mmap(
            tf.arg0().into(),
            tf.arg1() as _,
            tf.arg2() as _,
            tf.arg3() as _,
            tf.arg4() as _,
            tf.arg5() as _,
        ),
        Sysno::ioctl => sys_ioctl(tf.arg0() as _, tf.arg1() as _, tf.arg2().into()),
        Sysno::writev => sys_writev(tf.arg0() as _, tf.arg1().into(), tf.arg2() as _),
        Sysno::sched_yield => sys_sched_yield(),
        Sysno::nanosleep => sys_nanosleep(tf.arg0().into(), tf.arg1().into()),
        Sysno::getpid => sys_getpid(),
        Sysno::getppid => sys_getppid(),
        Sysno::exit => sys_exit(tf.arg0() as _),
        Sysno::gettimeofday => sys_get_time_of_day(tf.arg0().into()),
        Sysno::getcwd => sys_getcwd(tf.arg0().into(), tf.arg1() as _),
        Sysno::dup => sys_dup(tf.arg0() as _),
        Sysno::dup3 => sys_dup3(tf.arg0() as _, tf.arg1() as _),
        Sysno::fcntl => sys_fcntl(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
        Sysno::clone => sys_clone(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2().into(),
            tf.arg3() as _,
            tf.arg4().into(),
        ),
        #[cfg(any(target_arch = "x86_64", target_arch = "loongarch64"))]
        Sysno::clone => sys_clone(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2().into(),
            tf.arg3().into(),
            tf.arg4() as _,
        ),
        Sysno::wait4 => sys_wait4(tf.arg0() as _, tf.arg1().into(), tf.arg2() as _),
        Sysno::pipe2 => sys_pipe2(tf.arg0().into(), tf.arg1() as _),
        Sysno::close => sys_close(tf.arg0() as _),
        Sysno::chdir => sys_chdir(tf.arg0().into()),
        Sysno::execve => sys_execve(tf, tf.arg0().into(), tf.arg1().into(), tf.arg2().into()),
        Sysno::openat => sys_openat(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2() as _,
            tf.arg3() as _,
        ),
        #[cfg(target_arch = "x86_64")]
        Sysno::open => sys_open(tf.arg0().into(), tf.arg1() as _, tf.arg2() as _),
        Sysno::getdents64 => sys_getdents64(tf.arg0() as _, tf.arg1().into(), tf.arg2() as _),
        Sysno::linkat => sys_linkat(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2() as _,
            tf.arg3().into(),
            tf.arg4() as _,
        ),
        Sysno::uname => sys_uname(tf.arg0().into()),
        Sysno::fstat => sys_fstat(tf.arg0() as _, tf.arg1().into()),
        Sysno::mount => sys_mount(
            tf.arg0().into(),
            tf.arg1().into(),
            tf.arg2().into(),
            tf.arg3() as _,
            tf.arg4().into(),
        ) as _,
        Sysno::umount2 => sys_umount2(tf.arg0().into(), tf.arg1() as _) as _,
        #[cfg(target_arch = "x86_64")]
        Sysno::newfstatat => sys_fstatat(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2().into(),
            tf.arg3() as _,
        ),
        #[cfg(not(target_arch = "x86_64"))]
        Sysno::fstatat => sys_fstatat(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2().into(),
            tf.arg3() as _,
        ),
        Sysno::statx => sys_statx(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2() as _,
            tf.arg3() as _,
            tf.arg4().into(),
        ),
        Sysno::munmap => sys_munmap(tf.arg0().into(), tf.arg1() as _),
        Sysno::mprotect => sys_mprotect(tf.arg0().into(), tf.arg1() as _, tf.arg2() as _),
        Sysno::times => sys_times(tf.arg0().into()),
        Sysno::brk => sys_brk(tf.arg0() as _),
        #[cfg(target_arch = "x86_64")]
        Sysno::arch_prctl => sys_arch_prctl(tf.arg0() as _, tf.arg1().into(), tf),
        Sysno::set_tid_address => sys_set_tid_address(tf.arg0().into()),
        Sysno::clock_gettime => sys_clock_gettime(tf.arg0() as _, tf.arg1().into()),
        #[cfg(target_arch = "x86_64")]
        Sysno::dup2 => sys_dup3(tf.arg0() as _, tf.arg1() as _),
        Sysno::exit_group => sys_exit_group(tf.arg0() as _),
        #[cfg(target_arch = "x86_64")]
        Sysno::fork => sys_fork(),
        Sysno::futex => sys_futex(
            tf.arg0().into(),
            tf.arg1() as _,
            tf.arg2() as _,
            tf.arg3().into(),
            tf.arg4().into(),
            tf.arg5() as _,
        ),
        Sysno::fchmodat => sys_fchmodat(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2() as _,
            tf.arg3() as _,
        ),
        Sysno::getegid => sys_getegid(),
        Sysno::geteuid => sys_geteuid(),
        Sysno::getgid => sys_getgid(),
        Sysno::getrandom => sys_getrandom(tf.arg0().into(), tf.arg1() as _, tf.arg2() as _),
        Sysno::gettid => sys_gettid(),
        Sysno::getuid => sys_getuid(),
        Sysno::kill => sys_kill(tf.arg0() as _, tf.arg1() as _),
        Sysno::lseek => sys_lseek(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        #[cfg(target_arch = "x86_64")]
        Sysno::lstat => sys_lstat(tf.arg0().into(), tf.arg1().into()),
        #[cfg(target_arch = "x86_64")]
        Sysno::mkdir => sys_mkdir(tf.arg0().into(), tf.arg1() as _),
        Sysno::mkdirat => sys_mkdirat(tf.arg0() as _, tf.arg1().into(), tf.arg2() as _),
        #[cfg(target_arch = "x86_64")]
        Sysno::pipe => sys_pipe(tf.arg0().into()),
        #[cfg(target_arch = "x86_64")]
        Sysno::poll => sys_poll(tf.arg0().into(), tf.arg1() as _, tf.arg2() as _),
        Sysno::ppoll => sys_ppoll(
            tf.arg0().into(),
            tf.arg1() as _,
            tf.arg2().into(),
            tf.arg3().into(),
        ),
        Sysno::pread64 => sys_pread64(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2() as _,
            tf.arg3() as _,
        ),
        Sysno::prlimit64 => sys_prlimit64(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2().into(),
            tf.arg3().into(),
        ),
        Sysno::pselect6 => sys_pselect(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2().into(),
            tf.arg3().into(),
            tf.arg4().into(),
            tf.arg5().into(),
        ),
        Sysno::pwrite64 => sys_pwrite64(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2() as _,
            tf.arg3() as _,
        ),
        Sysno::setrlimit => sys_setrlimit(tf.arg0() as _, tf.arg1().into()),
        Sysno::getrlimit => sys_getrlimit(tf.arg0() as _, tf.arg1().into()),
        Sysno::readlinkat => sys_readlinkat(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2().into(),
            tf.arg3() as _,
        ),
        Sysno::readv => sys_readv(tf.arg0() as _, tf.arg1().into(), tf.arg2() as _),
        #[cfg(target_arch = "x86_64")]
        Sysno::rename => sys_rename(tf.arg0().into(), tf.arg1().into()),
        Sysno::renameat => sys_renameat(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2() as _,
            tf.arg3().into(),
        ),
        Sysno::renameat2 => sys_renameat2(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2() as _,
            tf.arg3().into(),
            tf.arg4() as _,
        ),
        #[cfg(target_arch = "x86_64")]
        Sysno::rmdir => sys_rmdir(tf.arg0().into()),
        Sysno::rt_sigaction => sys_rt_sigaction(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2().into(),
            tf.arg3() as _,
        ),
        Sysno::rt_sigpending => sys_rt_sigpending(tf.arg0().into(), tf.arg1() as _),
        Sysno::rt_sigprocmask => sys_rt_sigprocmask(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2().into(),
            tf.arg3() as _,
        ),
        Sysno::rt_sigqueueinfo => sys_rt_sigqueueinfo(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2().into(),
            tf.arg3() as _,
        ),
        Sysno::rt_sigreturn => sys_rt_sigreturn(tf),
        Sysno::rt_sigtimedwait => sys_rt_sigtimedwait(
            tf.arg0().into(),
            tf.arg1().into(),
            tf.arg2().into(),
            tf.arg3() as _,
        ),
        Sysno::rt_sigsuspend => sys_rt_sigsuspend(tf, tf.arg0().into(), tf.arg1() as _),
        Sysno::rt_tgsigqueueinfo => sys_rt_tgsigqueueinfo(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2() as _,
            tf.arg3().into(),
            tf.arg4() as _,
        ),
        #[cfg(target_arch = "x86_64")]
        Sysno::select => sys_select(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2().into(),
            tf.arg3().into(),
            tf.arg4().into(),
        ),
        Sysno::sendfile => sys_sendfile(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2().into(),
            tf.arg3() as _,
        ),
        Sysno::shmat => sys_shmat(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        Sysno::shmctl => sys_shmctl(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        Sysno::shmdt => sys_shmdt(tf.arg0() as _),
        Sysno::shmget => sys_shmget(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        Sysno::sigaltstack => sys_sigaltstack(tf.arg0().into(), tf.arg1().into()),
        #[cfg(target_arch = "x86_64")]
        Sysno::stat => sys_stat(tf.arg0().into(), tf.arg1().into()),
        Sysno::statfs => sys_statfs(tf.arg0().into(), tf.arg1().into()),
        Sysno::tgkill => sys_tgkill(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        Sysno::tkill => sys_tkill(tf.arg0() as _, tf.arg1() as _),
        #[cfg(target_arch = "x86_64")]
        Sysno::unlink => sys_unlink(tf.arg0().into()),
        Sysno::unlinkat => sys_unlinkat(tf.arg0() as _, tf.arg1().into(), tf.arg2() as _),
        Sysno::utimensat => sys_utimensat(
            tf.arg0() as _,
            tf.arg1().into(),
            tf.arg2().into(),
            tf.arg3() as _,
        ),
        Sysno::socket => sys_socket(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        Sysno::bind => sys_bind(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        Sysno::getsockname => sys_getsockname(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        Sysno::setsockopt => sys_setsockopt(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2() as _,
            tf.arg3() as _,
            tf.arg4() as _,
        ),
        Sysno::sendto => sys_sendto(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2() as _,
            tf.arg3() as _,
            tf.arg4() as _,
            tf.arg5() as _,
        ),
        Sysno::recvfrom => sys_recvfrom(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2() as _,
            tf.arg3() as _,
            tf.arg4() as _,
            tf.arg5() as _,
        ),
        Sysno::shutdown => sys_shutdown(tf.arg0() as _, tf.arg1() as _),
        Sysno::listen => sys_listen(tf.arg0() as _, tf.arg1() as _),
        Sysno::accept => sys_accept(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        Sysno::connect => sys_connect(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        #[cfg(target_arch = "x86_64")]
        Sysno::access => stub_bypass(syscall_num), // TODO: implement access
        Sysno::faccessat => sys_faccessat(tf.arg0() as _, tf.arg1().into(), tf.arg2() as _),
        Sysno::sync => stub_bypass(syscall_num),
        Sysno::fsync => stub_bypass(syscall_num),
        Sysno::truncate => sys_truncate(tf.arg0().into(), tf.arg1() as _),
        Sysno::ftruncate => sys_ftruncate(tf.arg0() as _, tf.arg1() as _),
        Sysno::syslog => stub_bypass(syscall_num),
        Sysno::get_robust_list => stub_bypass(syscall_num),
        Sysno::set_robust_list => stub_bypass(syscall_num),
        _ => stub_unimplemented(syscall_num),
    };
    let ans = result.unwrap_or_else(|err| -err.code() as _);
    time_stat_from_kernel_to_user();
    info!(
        "[syscall] <{:?}> return {}",
        Sysno::from(syscall_num as u32),
        ans
    );
    ans
}

fn stub_unimplemented(syscall_num: usize) -> Result<isize, LinuxError> {
    warn!(
        "Unimplemented syscall: {:?}, ENOSYS",
        Sysno::from(syscall_num as u32)
    );
    Err(LinuxError::ENOSYS)
}

fn stub_bypass(syscall_num: usize) -> Result<isize, LinuxError> {
    warn!(
        "Unimplemented syscall: {:?}, bypassed",
        Sysno::from(syscall_num as u32)
    );
    Ok(0)
}

fn stub_kill(syscall_num: usize) -> Result<isize, LinuxError> {
    warn!(
        "Unimplemented syscall: {:?}, killed",
        Sysno::from(syscall_num as u32)
    );
    sys_exit(0)
}
