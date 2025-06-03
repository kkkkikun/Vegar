use crate::ptr::{PtrWrapper, UserOutPtr};
use alloc::{sync::Arc, vec::Vec};
use axerrno::{LinuxError, LinuxResult};
use bitflags::bitflags;
use linux_raw_sys::general::{
    __WALL, __WCLONE, __WNOTHREAD, WCONTINUED, WEXITED, WNOHANG, WNOWAIT, WUNTRACED,
};
use starry_core::task::{current_process, current_process_data};
use syscall_trace::syscall_trace;
use undefined_process::Pid;
use undefined_process::process::Process;

bitflags! {
    #[derive(Debug)]
    struct WaitOptions: u32 {
        /// Do not block when there are no processes wishing to report status.
        const WNOHANG = WNOHANG;
        /// Report the status of selected processes which are stopped due to a
        /// `SIGTTIN`, `SIGTTOU`, `SIGTSTP`, or `SIGSTOP` signal.
        const WUNTRACED = WUNTRACED;
        /// Report the status of selected processes which have terminated.
        const WEXITED = WEXITED;
        /// Report the status of selected processes that have continued from a
        /// job control stop by receiving a `SIGCONT` signal.
        const WCONTINUED = WCONTINUED;
        /// Don't reap, just poll status.
        const WNOWAIT = WNOWAIT;

        /// Don't wait on children of other threads in this group
        const WNOTHREAD = __WNOTHREAD;
        /// Wait on all children, regardless of type
        const WALL = __WALL;
        /// Wait for "clone" children only.
        const WCLONE = __WCLONE;
    }
}

#[derive(Debug, Clone, Copy)]
enum WaitPid {
    /// Wait for any child process
    Any,
    /// Wait for the child whose process ID is equal to the value.
    Pid(Pid),
    /// Wait for any child process whose process group ID is equal to the value.
    Pgid(Pid),
}

impl WaitPid {
    fn apply(&self, child: &Arc<Process>) -> bool {
        match self {
            WaitPid::Any => true,
            WaitPid::Pid(pid) => child.get_pid() == *pid,
            WaitPid::Pgid(pgid) => child.get_group().get_pgid() == *pgid,
        }
    }
}

#[syscall_trace]
pub fn sys_wait4(pid: i32, exit_code_ptr: UserOutPtr<i32>, options: u32) -> LinuxResult<isize> {
    let options = WaitOptions::from_bits_truncate(options);
    info!("sys_waitpid <= pid: {:?}, options: {:?}", pid, options);

    let process = current_process();
    let process_data = current_process_data();

    let pid = if pid == -1 {
        WaitPid::Any
    } else if pid == 0 {
        WaitPid::Pgid(process.get_group().get_pgid())
    } else if pid > 0 {
        WaitPid::Pid(pid as _)
    } else {
        WaitPid::Pgid(-pid as _)
    };

    let children = process
        .get_children()
        .into_iter()
        .filter(|child| pid.apply(child))
        // .filter(|child| {
        //     options.contains(WaitOptions::WALL)
        //         || (options.contains(WaitOptions::WCLONE)
        //             == get_process_data(child.get_pid()).is_some_and(|x| x.is_clone_child()))
        // })
        .collect::<Vec<_>>();
    if children.is_empty() {
        return Err(LinuxError::ECHILD);
    }

    let exit_code = exit_code_ptr.get();
    loop {
        if let Some(child) = children.iter().find(|child| child.is_zombie()) {
            if !options.contains(WaitOptions::WNOWAIT) {
                child.release();
            }
            if let Ok(exit_code) = exit_code {
                // TODO: other exit signals (low 8 bits)
                unsafe {
                    *exit_code = child.get_exit_code() << 8;
                }
            }
            return Ok(child.get_pid() as _);
        } else if options.contains(WaitOptions::WNOHANG) {
            return Ok(0);
        } else {
            // signal
            process_data.child_exit_wq.wait();
        }
    }
}
