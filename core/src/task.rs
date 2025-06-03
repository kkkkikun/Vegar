use core::{alloc::Layout, sync::atomic::Ordering};

use crate::ctypes::TimeStat;
use crate::process::{ProcessData, ThreadData};
use alloc::{string::String, sync::Arc};
use axhal::{
    arch::{TrapFrame, UspaceContext},
    time::{NANOS_PER_MICROS, NANOS_PER_SEC, monotonic_time_nanos},
};
use axns::{AxNamespace, AxNamespaceIf};
use axtask::{TaskExtRef, TaskInner, WaitQueue, current};
use core::cell::RefCell;
use core::time::Duration;
use spin::Once;
use undefined_process::process::Process;
use undefined_process::thread::Thread;

/// Task extended data for the monolithic kernel.
pub struct TaskExt {
    /// The time statistics.
    pub time: RefCell<TimeStat>,
    /// The POSIX thread corresponding to this task.
    pub thread: Arc<Thread>,
    /// The thread data bind to this task.
    pub thread_data: Arc<ThreadData>,
}

pub fn current_thread() -> Arc<Thread> {
    current().task_ext().thread.clone()
}

pub fn current_thread_data() -> Arc<ThreadData> {
    current().task_ext().thread_data.clone()
}

pub fn current_process() -> Arc<Process> {
    current_thread().get_process()
}

pub fn current_process_data() -> Arc<ProcessData> {
    current_thread_data().process_data.clone()
}

impl TaskExt {
    pub fn new(thread: Arc<Thread>, thread_data: Arc<ThreadData>) -> Self {
        Self {
            time: TimeStat::new().into(),
            thread,
            thread_data,
        }
    }

    pub(crate) fn time_stat_from_kernel_to_user(&self, current_tick: usize) {
        self.time.borrow_mut().switch_into_user_mode(current_tick);
    }

    pub(crate) fn time_stat_from_user_to_kernel(&self, current_tick: usize) {
        self.time.borrow_mut().switch_into_kernel_mode(current_tick);
    }

    pub(crate) fn time_stat_output(&self) -> (usize, usize) {
        self.time.borrow().output()
    }
}

struct AxNamespaceImpl;
#[crate_interface::impl_interface]
impl AxNamespaceIf for AxNamespaceImpl {
    fn current_namespace_base() -> *mut u8 {
        // Namespace for kernel task
        static KERNEL_NS_BASE: Once<usize> = Once::new();
        let current = axtask::current();
        // Safety: We only check whether the task extended data is null and do not access it.
        if unsafe { current.task_ext_ptr() }.is_null() {
            return *(KERNEL_NS_BASE.call_once(|| {
                let global_ns = AxNamespace::global();
                let layout = Layout::from_size_align(global_ns.size(), 64).unwrap();
                // Safety: The global namespace is a static readonly variable and will not be dropped.
                let dst = unsafe { alloc::alloc::alloc(layout) };
                let src = global_ns.base();
                unsafe { core::ptr::copy_nonoverlapping(src, dst, global_ns.size()) };
                dst as usize
            })) as *mut u8;
        }
        current_thread_data().namespace.base()
    }
}

impl Drop for TaskExt {
    fn drop(&mut self) {
        trace!("TaskExt drop.");
    }
}

axtask::def_task_ext!(TaskExt);

#[allow(unused)]
pub fn write_trapframe_to_kstack(kstack_top: usize, trap_frame: &TrapFrame) {
    let trap_frame_size = core::mem::size_of::<TrapFrame>();
    let trap_frame_ptr = (kstack_top - trap_frame_size) as *mut TrapFrame;
    unsafe {
        *trap_frame_ptr = *trap_frame;
    }
}

pub fn read_trapframe_from_kstack(kstack_top: usize) -> TrapFrame {
    let trap_frame_size = core::mem::size_of::<TrapFrame>();
    let trap_frame_ptr = (kstack_top - trap_frame_size) as *mut TrapFrame;
    unsafe { *trap_frame_ptr }
}

pub fn time_stat_from_kernel_to_user() {
    let curr_task = current();
    curr_task
        .task_ext()
        .time_stat_from_kernel_to_user(monotonic_time_nanos() as usize);
}

pub fn time_stat_from_user_to_kernel() {
    let curr_task = current();
    curr_task
        .task_ext()
        .time_stat_from_user_to_kernel(monotonic_time_nanos() as usize);
}

pub fn time_stat_output() -> (usize, usize, usize, usize) {
    let curr_task = current();
    let (utime_ns, stime_ns) = curr_task.task_ext().time_stat_output();
    (
        utime_ns / NANOS_PER_SEC as usize,
        utime_ns / NANOS_PER_MICROS as usize,
        stime_ns / NANOS_PER_SEC as usize,
        stime_ns / NANOS_PER_MICROS as usize,
    )
}

pub fn create_user_task(name: String, uctx: UspaceContext) -> TaskInner {
    let name = name.chars().take(32).collect();
    TaskInner::new(
        move || {
            let curr = current();
            let kstack_top = curr.kernel_stack_top().unwrap();
            info!(
                "Enter user space: entry={:#x}, ustack={:#x}, kstack={:#x}",
                uctx.ip(),
                uctx.sp(),
                kstack_top,
            );

            // Set the tid into address `set_child_tid`:
            // When `set_child_tid` is set, the very first thing
            // the new thread does is to write its thread ID at this address.
            let addr = current_thread_data()
                .addr_set_child_tid
                .load(Ordering::Relaxed);
            // TODO: user UserPtr wrapper to check it
            // however, UserPtr is not defined in current crate
            let addr: *mut u32 = addr as _;
            info!("set_child_tid={:#x}", addr.addr());
            if !addr.is_null() {
                unsafe { addr.write(current_thread().get_tid()) };
            }

            unsafe { uctx.enter_uspace(kstack_top) }
        },
        name,
        axconfig::plat::KERNEL_STACK_SIZE,
    )
}

#[doc(hidden)]
pub struct WaitQueueWrapper(WaitQueue);
impl Default for WaitQueueWrapper {
    fn default() -> Self {
        Self(WaitQueue::new())
    }
}
impl axsignal::api::WaitQueue for WaitQueueWrapper {
    fn wait_timeout(&self, timeout: Option<Duration>) -> bool {
        if let Some(timeout) = timeout {
            self.0.wait_timeout(timeout, false)
        } else {
            self.0.wait();
            true
        }
    }

    fn notify_one(&self) -> bool {
        self.0.notify_one(false)
    }
}
