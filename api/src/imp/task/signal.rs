use core::{mem, time::Duration};

use arceos_posix_api::ctypes::timespec;
use axerrno::{LinuxError, LinuxResult};
use linux_raw_sys::general::{
    MINSIGSTKSZ, SI_TKILL, SI_USER, SIG_BLOCK, SIG_SETMASK, SIG_UNBLOCK, kernel_sigaction, siginfo,
};

use crate::ptr::{PtrWrapper, UserConstPtr, UserPtr};

use crate::imp::task::sys_exit_impl;
use axhal::{
    arch::TrapFrame,
    trap::{POST_TRAP, register_trap_handler},
};
use axsignal::{SignalInfo, SignalOSAction, SignalSet, SignalStack, Signo};
use starry_core::process::{get_process_data, get_thread_data};
use starry_core::task::{current_process, current_process_data, current_thread_data};
use undefined_process::Pid;
use undefined_process::process::get_all_processes;
use undefined_process::process_group::get_process_group;
use undefined_process::thread::get_thread;

fn check_signals(tf: &mut TrapFrame, restore_blocked: Option<SignalSet>) -> bool {
    let signal = &current_thread_data().signal;
    let Some((sig, os_action)) = signal.check_signals(tf, restore_blocked) else {
        return false;
    };

    let signo = sig.signo();
    match os_action {
        SignalOSAction::Terminate => {
            sys_exit_impl(128 + signo as i32, true);
        }
        SignalOSAction::CoreDump => {
            // TODO: implement core dump
            sys_exit_impl(128 + signo as i32, true);
        }
        SignalOSAction::Stop => {
            // TODO: implement stop
            sys_exit_impl(1, true);
        }
        SignalOSAction::Continue => {
            // TODO: implement continue
        }
        SignalOSAction::Handler => {
            // do nothing
        }
    }
    true
}

#[register_trap_handler(POST_TRAP)]
fn post_trap_callback(tf: &mut TrapFrame, from_user: bool) {
    if !from_user {
        return;
    }

    check_signals(tf, None);
}

fn check_sigset_size(size: usize) -> LinuxResult<()> {
    if size != size_of::<SignalSet>() {
        return Err(LinuxError::EINVAL);
    }
    Ok(())
}

fn parse_signo(signo: u32) -> LinuxResult<Signo> {
    Signo::from_repr(signo as u8).ok_or(LinuxError::EINVAL)
}

pub fn sys_rt_sigprocmask(
    how: i32,
    set: UserConstPtr<SignalSet>,
    oldset: UserPtr<SignalSet>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    current_thread_data()
        .signal
        .with_blocked_mut::<LinuxResult<_>>(|blocked| {
            if let Some(oldset) = oldset.nullable(UserPtr::get)? {
                unsafe { *oldset = *blocked };
            }

            if let Some(set) = set.nullable(UserConstPtr::get)? {
                let set = unsafe { *set };
                match how as u32 {
                    SIG_BLOCK => *blocked |= set,
                    SIG_UNBLOCK => *blocked &= !set,
                    SIG_SETMASK => *blocked = set,
                    _ => return Err(LinuxError::EINVAL),
                }
            }
            Ok(())
        })?;

    Ok(0)
}

pub fn sys_rt_sigaction(
    signo: u32,
    act: UserConstPtr<kernel_sigaction>,
    oldact: UserPtr<kernel_sigaction>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let signo = parse_signo(signo)?;
    if matches!(signo, Signo::SIGKILL | Signo::SIGSTOP) {
        return Err(LinuxError::EINVAL);
    }

    let signal = &current_process_data().signal;
    let mut actions = signal.actions.lock();
    if let Some(oldact) = oldact.nullable(UserPtr::get)? {
        actions[signo].to_ctype(unsafe { &mut *oldact });
    }
    if let Some(act) = act.nullable(UserConstPtr::get)? {
        actions[signo] = unsafe { (*act).try_into()? };
    }
    Ok(0)
}

pub fn sys_rt_sigpending(set: UserPtr<SignalSet>, sigsetsize: usize) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;
    unsafe {
        *set.get()? = current_thread_data().signal.pending();
    }
    Ok(0)
}

pub fn send_signal_thread(tid: Pid, sig: SignalInfo) -> LinuxResult<()> {
    info!("Send signal {:?} to thread {}", sig.signo(), tid);
    let thread_data = get_thread_data(tid).ok_or(LinuxError::EPERM)?;
    thread_data.signal.send_signal(sig);
    Ok(())
}
pub fn send_signal_process(pid: Pid, sig: SignalInfo) -> LinuxResult<()> {
    info!("Send signal {:?} to process {}", sig.signo(), pid);
    let process_data = get_process_data(pid).ok_or(LinuxError::EPERM)?;
    process_data.signal.send_signal(sig);
    Ok(())
}
pub fn send_signal_process_group(pgid: Pid, sig: SignalInfo) -> usize {
    info!("Send signal {:?} to process group {}", sig.signo(), pgid);
    let mut count = 0;
    let Some(pg) = get_process_group(pgid) else {
        return 0;
    };
    for process in pg.get_processes() {
        count += send_signal_process(process.get_pid(), sig.clone()).is_ok() as usize;
    }
    count
}

fn make_siginfo(signo: u32, code: u32) -> LinuxResult<Option<SignalInfo>> {
    if signo == 0 {
        return Ok(None);
    }
    let signo = parse_signo(signo)?;
    Ok(Some(SignalInfo::new(signo, code)))
}

pub fn sys_kill(pid: i32, signo: u32) -> LinuxResult<isize> {
    let Some(sig) = make_siginfo(signo, SI_USER)? else {
        // TODO: should also check permissions
        return Ok(0);
    };

    let mut result = 0usize;
    match pid {
        1.. => {
            send_signal_process(pid as _, sig)?;
            result += 1;
        }
        0 => {
            let pg = current_process().get_group();
            result += send_signal_process_group(pg.get_pgid(), sig);
        }
        -1 => {
            for process in get_all_processes() {
                // TODO: skip init process?
                send_signal_process(process.get_pid(), sig.clone())?;
                result += 1;
            }
        }
        ..-1 => {
            let pg = get_process_group((-pid) as Pid).ok_or(LinuxError::ESRCH)?;
            result += send_signal_process_group(pg.get_pgid(), sig);
        }
    }

    debug!("[sys_kill] successfully sent signal {} processes", result);

    if result > 0 {
        Ok(0)
    } else {
        Err(LinuxError::ESRCH)
    }
}

pub fn sys_tkill(tid: Pid, signo: u32) -> LinuxResult<isize> {
    let Some(sig) = make_siginfo(signo, SI_TKILL as u32)? else {
        // TODO: should also check permissions
        return Ok(0);
    };

    send_signal_thread(tid, sig)?;
    Ok(0)
}

pub fn sys_tgkill(tgid: Pid, tid: Pid, signo: u32) -> LinuxResult<isize> {
    let Some(sig) = make_siginfo(signo, SI_TKILL as u32)? else {
        // TODO: should also check permissions
        return Ok(0);
    };
    check_thread(tid, tgid)?;
    send_signal_thread(tid, sig)?;
    Ok(0)
}

// check if the thread belongs to the same thread group (process)
fn check_thread(tid: Pid, tgid: Pid) -> LinuxResult<()> {
    let thread = get_thread(tid).ok_or(LinuxError::ESRCH)?;
    if thread.get_process().get_pid() != tgid {
        return Err(LinuxError::ESRCH);
    }
    Ok(())
}

fn make_queue_signal_info(
    tgid: Pid,
    signo: u32,
    sig: UserConstPtr<SignalInfo>,
) -> LinuxResult<SignalInfo> {
    let signo = parse_signo(signo)?;
    let mut sig = unsafe { sig.get()?.read() };
    sig.set_signo(signo);
    if sig.code() != SI_USER && current_process().get_pid() != tgid {
        return Err(LinuxError::EPERM);
    }
    Ok(sig)
}

pub fn sys_rt_sigqueueinfo(
    tgid: Pid,
    signo: u32,
    sig: UserConstPtr<SignalInfo>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let sig = make_queue_signal_info(tgid, signo, sig)?;
    send_signal_process(tgid, sig)?;
    Ok(0)
}

pub fn sys_rt_tgsigqueueinfo(
    tgid: Pid,
    tid: Pid,
    signo: u32,
    sig: UserConstPtr<SignalInfo>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    check_thread(tid, tgid)?;
    let sig = make_queue_signal_info(tgid, signo, sig)?;
    send_signal_thread(tid, sig)?;
    Ok(0)
}

pub fn sys_rt_sigreturn(tf: &mut TrapFrame) -> LinuxResult<isize> {
    current_thread_data().signal.restore(tf);
    Ok(tf.retval() as isize)
}

pub fn sys_rt_sigtimedwait(
    set: UserConstPtr<SignalSet>,
    info: UserPtr<siginfo>,
    timeout: UserConstPtr<timespec>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let set = unsafe { *set.get()? };
    let timeout: Option<Duration> = timeout
        .nullable(UserConstPtr::get)?
        .map(|ts| unsafe { *ts }.into());

    let Some(sig) = current_thread_data().signal.wait_timeout(set, timeout) else {
        return Err(LinuxError::EAGAIN);
    };

    if let Some(info) = info.nullable(UserPtr::get)? {
        unsafe { *info = sig.0 };
    }

    Ok(0)
}

pub fn sys_rt_sigsuspend(
    tf: &mut TrapFrame,
    set: UserPtr<SignalSet>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let thr_data = current_thread_data();
    let mut set = unsafe { *set.get()? };

    set.remove(Signo::SIGKILL);
    set.remove(Signo::SIGSTOP);

    let old_blocked = thr_data
        .signal
        .with_blocked_mut(|blocked| mem::replace(blocked, set));

    tf.set_retval((-LinuxError::EINTR.code() as isize) as usize);

    loop {
        if check_signals(tf, Some(old_blocked)) {
            break;
        }
        current_process_data().signal.wait_signal();
    }

    Ok(0)
}

pub fn sys_sigaltstack(
    ss: UserConstPtr<SignalStack>,
    old_ss: UserPtr<SignalStack>,
) -> LinuxResult<isize> {
    current_thread_data().signal.with_stack_mut(|stack| {
        if let Some(old_ss) = old_ss.nullable(UserPtr::get)? {
            unsafe { *old_ss = stack.clone() };
        }
        if let Some(ss) = ss.nullable(UserConstPtr::get)? {
            let ss = unsafe { ss.read() };
            if ss.size <= MINSIGSTKSZ as usize {
                return Err(LinuxError::ENOMEM);
            }
            let stack_ptr: UserConstPtr<u8> = ss.sp.into();
            let _ = stack_ptr.get_as_array(ss.size)?;

            *stack = ss.clone();
        }
        Ok(0)
    })
}
