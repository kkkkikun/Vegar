use crate::imp::task::signal::{send_signal_process, send_signal_thread};
use crate::ptr::{PtrWrapper, UserPtr};
use arceos_posix_api::close_all_file_like;
use axsignal::{SignalInfo, Signo};
use core::sync::atomic::Ordering;
use linux_raw_sys::general::SI_KERNEL;
use starry_core::process::get_process_data;
use starry_core::task::{
    current_process, current_process_data, current_thread, current_thread_data,
};
use undefined_process::Pid;

pub fn sys_exit_impl(exit_code: i32, exit_group: bool) -> ! {
    {
        if exit_group {
            info!(
                "[exit] process {} exiting with code {}",
                current_process().get_pid(),
                exit_code
            );
        } else {
            info!(
                "[exit] thread {} exiting with code {}",
                current_thread().get_tid(),
                exit_code
            );
        }
        let addr_clear_child_tid = current_thread_data()
            .addr_clear_child_tid
            .load(Ordering::Relaxed);
        let addr_clear_child_tid = UserPtr::<Pid>::from(addr_clear_child_tid);
        if let Ok(ptr) = addr_clear_child_tid.get() {
            unsafe { ptr.write(0) };
            // TODO: wake up threads, which are blocked by futex, and waiting for the address pointed by clear_child_tid
            let table = &current_process_data().futex_table;
            let addr = addr_clear_child_tid.address().as_usize();
            table.lock().get(&addr).cloned().map(|futex| {
                debug!("wake up futex");
                futex.notify_all(false);
            });
            axtask::yield_now();
        }
        current_thread().exit(exit_code);
        let process = current_process();
        if process.is_zombie() {
            // threads have exited
            // send signals
            if let Some(parent) = process.get_parent() {
                if let Some(parent_data) = get_process_data(parent.get_pid()) {
                    let signal = parent_data.exit_signal.unwrap_or(Signo::SIGCHLD);
                    let _ =
                        send_signal_process(parent.get_pid(), SignalInfo::new(signal, SI_KERNEL));
                    parent_data.child_exit_wq.notify_all(false)
                }
            }
        }
        // release thread data
        // FIXME: leak of ax-namespace
        close_all_file_like();
        if exit_group {
            // TODO: prevent exit_group from being called multiple times
            let sig = SignalInfo::new(Signo::SIGKILL, SI_KERNEL);
            for thread in process.get_threads() {
                let _ = send_signal_thread(thread.get_tid(), sig.clone());
            }
        }
    }
    axtask::exit(exit_code)
}
