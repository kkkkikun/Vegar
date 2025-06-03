use alloc::string::ToString;
use alloc::sync::Arc;
use arceos_posix_api::FD_TABLE;
use axerrno::{LinuxError, LinuxResult};
use axfs::{CURRENT_DIR, CURRENT_DIR_PATH};
use axhal::arch::UspaceContext;
use axsignal::Signo;
use axtask::current;
use bitflags::bitflags;
use core::sync::atomic::Ordering;
use linux_raw_sys::general::*;
use spin::Mutex;
use starry_core::mm::copy_from_kernel;
use starry_core::process::{ProcessData, create_thread_data, get_process_data};
use starry_core::task::{
    TaskExt, create_user_task, current_process, current_process_data, read_trapframe_from_kstack,
};

bitflags! {
    /// Options for use with [`sys_clone`].
    #[derive(Debug, Clone, Copy, Default)]
    pub struct CloneFlags: u32 {
        /// The calling process and the child process run in the same
        /// memory space.
        const VM = CLONE_VM;
        /// The caller and the child process share the same  filesystem
        /// information.
        const FS = CLONE_FS;
        /// The calling process and the child process share the same file
        /// descriptor table.
        const FILES = CLONE_FILES;
        /// The calling process and the child process share the same table
        /// of signal handlers.
        const SIGHAND = CLONE_SIGHAND;
        /// If the calling process is being traced, then trace the child
        /// also.
        const PTRACE = CLONE_PTRACE;
        /// The execution of the calling process is suspended until the
        /// child releases its virtual memory resources via a call to
        /// execve(2) or _exit(2) (as with vfork(2)).
        const VFORK = CLONE_VFORK;
        /// The parent of the new child  (as returned by getppid(2))
        /// will be the same as that of the calling process.
        const PARENT = CLONE_PARENT;
        /// The child is placed in the same thread group as the calling
        /// process.
        const THREAD = CLONE_THREAD;
        /// The cloned child is started in a new mount namespace.
        const NEWNS = CLONE_NEWNS;
        /// The child and the calling process share a single list of System
        /// V semaphore adjustment values
        const SYSVSEM = CLONE_SYSVSEM;
        /// The TLS (Thread Local Storage) descriptor is set to tls.
        const SETTLS = CLONE_SETTLS;
        /// Store the child thread ID in the parent's memory.
        const PARENT_SETTID = CLONE_PARENT_SETTID;
        /// Clear (zero) the child thread ID in child memory when the child
        /// exits, and do a wakeup on the futex at that address.
        const CHILD_CLEARTID = CLONE_CHILD_CLEARTID;
        /// A tracing process cannot force `CLONE_PTRACE` on this child
        /// process.
        const UNTRACED = CLONE_UNTRACED;
        /// Store the child thread ID in the child's memory.
        const CHILD_SETTID = CLONE_CHILD_SETTID;
        /// Create the process in a new cgroup namespace.
        const NEWCGROUP = CLONE_NEWCGROUP;
        /// Create the process in a new UTS namespace.
        const NEWUTS = CLONE_NEWUTS;
        /// Create the process in a new IPC namespace.
        const NEWIPC = CLONE_NEWIPC;
        /// Create the process in a new user namespace.
        const NEWUSER = CLONE_NEWUSER;
        /// Create the process in a new PID namespace.
        const NEWPID = CLONE_NEWPID;
        /// Create the process in a new network namespace.
        const NEWNET = CLONE_NEWNET;
        /// The new process shares an I/O context with the calling process.
        const IO = CLONE_IO;
    }
}

pub fn sys_clone_impl(
    clone_flags: CloneFlags,
    new_sp: usize,
    tls: usize,
    addr_child_tid: usize,
    exit_signal: Option<Signo>,
) -> LinuxResult<isize> {
    // duplicate trap frame
    let trap_frame = read_trapframe_from_kstack(current().get_kernel_stack_top().unwrap());
    let mut new_uctx = UspaceContext::from(&trap_frame);
    // set user stack
    // If new_sp == 0, the child uses a duplicate of the parent's stack.
    // (Copy-on-write semantics ensure that the child gets separate copies of stack pages
    // when either process modifies the stack.)
    // In this case, for correct operation, the CLONE_VM option should not be specified.
    // (If the child shares the parent's memory because of the use of the CLONE_VM flag,
    // then no copy-on-write duplication occurs and chaos is likely to result.)
    if new_sp != 0 {
        new_uctx.set_sp(new_sp);
    }
    // set return value of the new thread(task)
    new_uctx.set_retval(0);
    // set thread local storage
    if clone_flags.contains(CloneFlags::SETTLS) {
        new_uctx.set_tls(tls.into());
    }

    // create user task for scheduler
    let name = current().name().to_string() + "_";
    let mut new_task = create_user_task(name, new_uctx);

    // init task extended data
    let (thread, thread_data) = if clone_flags.contains(CloneFlags::THREAD) {
        // create thread
        // clone address space
        let page_table = current_process_data().addr_space.lock().page_table_root();
        new_task.ctx_mut().set_page_table_root(page_table);

        let thread = current_process().create_thread();
        let thread_data = create_thread_data(current_process_data().clone(), thread.get_tid());
        // signals
        // for thread, there should be no exit_signal,
        // and there must be a CLONE_SIGHAND flag
        // so we needn't do anything here
        (thread, thread_data)
    } else {
        // create process
        // construct process data
        // address space
        let addr_space =
            if clone_flags.contains(CloneFlags::VM) && !clone_flags.contains(CloneFlags::VFORK) {
                // create another reference to the same address space
                // we clone the `Arc` itself rather than the data
                current_process_data().addr_space.clone()
            } else {
                // clone the address space
                let addr_space = &current_process_data().addr_space;
                let mut addr_space = addr_space.lock();
                let mut new_addr_space = addr_space.clone_or_err()?;
                copy_from_kernel(&mut new_addr_space)?;
                Arc::new(Mutex::new(new_addr_space))
            };
        let page_table = addr_space.lock().page_table_root();
        new_task.ctx_mut().set_page_table_root(page_table);
        // parent
        let parent = if clone_flags.contains(CloneFlags::PARENT) {
            current_process().get_parent().ok_or(LinuxError::EINVAL)?
        } else {
            current_process()
        };
        // signals
        let signal_actions = if clone_flags.contains(CloneFlags::SIGHAND) {
            let parent_data = get_process_data(parent.get_pid()).unwrap();
            parent_data.signal.actions.clone()
        } else {
            Arc::default()
        };
        // fork new process
        let new_process = parent.fork();
        let new_thread = new_process.get_main_thread().unwrap();
        let process_data = ProcessData::new(
            current_process_data().command_line.lock().clone(),
            addr_space,
            signal_actions,
            exit_signal,
        );
        let thread_data = create_thread_data(Arc::new(process_data), new_thread.get_tid());

        (new_thread, thread_data)
    };

    // share or create process/thread data
    if clone_flags.contains(CloneFlags::FILES) {
        FD_TABLE
            .deref_from(&thread_data.namespace)
            .init_shared(FD_TABLE.share());
    } else {
        FD_TABLE
            .deref_from(&thread_data.namespace)
            .init_new(FD_TABLE.copy_inner());
    }

    if clone_flags.contains(CloneFlags::FS) {
        CURRENT_DIR
            .deref_from(&thread_data.namespace)
            .init_shared(CURRENT_DIR.share());
        CURRENT_DIR_PATH
            .deref_from(&thread_data.namespace)
            .init_shared(CURRENT_DIR_PATH.share());
    } else {
        CURRENT_DIR
            .deref_from(&thread_data.namespace)
            .init_new(CURRENT_DIR.copy_inner());
        CURRENT_DIR_PATH
            .deref_from(&thread_data.namespace)
            .init_new(CURRENT_DIR_PATH.copy_inner());
    }

    if clone_flags.contains(CloneFlags::CHILD_SETTID) {
        thread_data
            .addr_set_child_tid
            .store(addr_child_tid, Ordering::Relaxed);
    }

    if clone_flags.contains(CloneFlags::CHILD_CLEARTID) {
        thread_data
            .addr_clear_child_tid
            .store(addr_child_tid, Ordering::Relaxed);
    }

    // create `TaskExt`
    let tid = thread.get_tid();
    new_task.init_task_ext(TaskExt::new(thread, thread_data));

    // spawn the task
    axtask::spawn_task(new_task);

    // return the thread id of the new thread
    Ok(tid as _)
}
