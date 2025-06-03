use crate::Pid;
use crate::process::Process;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use spin::Mutex;

pub struct Thread {
    tid: Pid,
    process: Weak<Process>,
}

impl Thread {
    pub fn get_tid(&self) -> Pid {
        self.tid
    }

    pub fn get_process(&self) -> Arc<Process> {
        self.process.upgrade().unwrap()
    }

    /// Check if the thread is the main thread of the process.
    /// The "main thread" is also known as the "leader" of the thread group.
    /// If any of the threads in a thread group performs an execve,
    /// then all threads other than the thread group leader are terminated,
    /// and the new program is executed in the thread group leader.
    pub fn is_main_thread(&self) -> bool {
        self.tid == self.get_process().get_pid()
    }

    pub fn exit(self: &Arc<Self>, exit_code: i32) {
        self.get_process().remove_thread(self.tid, exit_code);
        THREAD_TABLE.lock().remove(&self.tid);
    }

    fn new(tid: Pid, process: Weak<Process>) -> Arc<Self> {
        Arc::new(Self { tid, process })
    }
}

static THREAD_TABLE: Mutex<BTreeMap<Pid, Arc<Thread>>> =
    Mutex::new(BTreeMap::<Pid, Arc<Thread>>::new());

/// Create a new thread if the thread does not exist
/// The new thread will be added to the process
pub(crate) fn create_thread(tid: Pid, process: Weak<Process>) -> Arc<Thread> {
    let mut thread_table = THREAD_TABLE.lock();
    if thread_table.contains_key(&tid) {
        panic!("[process] thread with id {} already exists", tid);
    }
    let thread = Thread::new(tid, process.clone());
    let process = process.upgrade().unwrap();
    process.add_thread(thread.clone());
    thread_table.insert(tid, thread.clone());
    thread
}

pub fn get_thread(tid: Pid) -> Option<Arc<Thread>> {
    let thread_table = THREAD_TABLE.lock();
    thread_table.get(&tid).cloned()
}
