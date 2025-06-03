use crate::Pid;
use crate::process_group::{ProcessGroup, create_process_group};
use crate::session::{Session, create_session};
use crate::thread::{Thread, create_thread};
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use spin::Mutex;

pub struct Process {
    pid: Pid,
    threads: Mutex<BTreeMap<Pid, Arc<Thread>>>,
    process_group: Mutex<Weak<ProcessGroup>>,
    children: Mutex<BTreeMap<Pid, Arc<Process>>>,
    parent: Mutex<Weak<Process>>,
    is_zombie: AtomicBool,
    exit_code: AtomicI32,
    // TODO: sub reaper
}

impl Process {
    pub fn get_pid(&self) -> Pid {
        self.pid
    }

    pub fn get_group(&self) -> Arc<ProcessGroup> {
        self.process_group.lock().upgrade().unwrap()
    }

    pub fn get_session(&self) -> Arc<Session> {
        self.get_group().get_session()
    }

    pub fn is_group_leader(&self) -> bool {
        self.get_group().get_pgid() == self.pid
    }

    pub fn is_session_leader(&self) -> bool {
        self.get_session().get_sid() == self.pid
    }

    fn change_group(self: &Arc<Self>, new_group: &Arc<ProcessGroup>) {
        let origin_group = self.get_group();
        origin_group.remove_process(self.pid);
        new_group.add_process(self.clone());
        *self.process_group.lock() = Arc::downgrade(&new_group);
    }

    /// Create a new process group if the current process is not the leader of a group
    /// Return the new process group, or the current process group if the current process is the leader
    pub fn create_group(self: &Arc<Self>) -> Arc<ProcessGroup> {
        let origin_group = self.get_group();
        if origin_group.get_pgid() == self.pid {
            // if current process is the leader of the group, return the current group
            return origin_group;
        }
        // create a new process group with current pid
        let new_group = create_process_group(self.pid, origin_group.session.clone());
        self.change_group(&new_group);
        new_group
    }

    pub fn move_to_group(self: &Arc<Self>, new_group: Pid) -> bool {
        if self.is_session_leader() {
            // session leader cannot move to another group
            return false;
        }
        let session = self.get_session();
        let new_group = session.get_process_group(new_group);
        if new_group.is_none() {
            // new group does not exist,
            // or attempts to move a process into a process group in a different session
            return false;
        }
        self.change_group(&new_group.unwrap());
        true
    }

    /// Create a new session if the current process is not the leader of a session.
    /// Current process will become the leader of the new session, and the process group leader of
    /// a new process group in the session.
    pub fn create_session(self: &Arc<Self>) -> Option<Arc<Session>> {
        if self.is_group_leader() {
            // if current process is the leader of the group, fails
            return None;
        }
        // create a new session with current pid
        let new_session = create_session(self.pid);
        let new_group = create_process_group(self.pid, Arc::downgrade(&new_session));
        self.change_group(&new_group);
        Some(new_session)
    }

    pub fn get_parent(&self) -> Option<Arc<Process>> {
        self.parent.lock().upgrade()
    }

    pub fn get_children(&self) -> Vec<Arc<Process>> {
        self.children.lock().values().cloned().collect()
    }

    /// only can be used in `create_process` function
    /// does nothing but initialize fields
    fn new(pid: Pid, parent: Weak<Process>, group: Weak<ProcessGroup>) -> Arc<Self> {
        Arc::new(Self {
            pid,
            threads: Mutex::new(BTreeMap::new()),
            process_group: Mutex::new(group),
            children: Mutex::new(BTreeMap::new()),
            parent: Mutex::new(parent),
            is_zombie: AtomicBool::new(false),
            exit_code: AtomicI32::new(0),
        })
    }

    /// only used to spawn a "newborn" process without parent, like `init` process
    pub fn spawn_process() -> Arc<Process> {
        let pid = generate_next_pid();
        let new_session = create_session(pid);
        let new_group = create_process_group(pid, Arc::downgrade(&new_session));
        create_process(pid, Weak::new(), Arc::downgrade(&new_group))
    }

    pub fn fork(self: &Arc<Self>) -> Arc<Process> {
        let pid = generate_next_pid();
        let new_group = self.get_group();
        let new_process = create_process(pid, Arc::downgrade(self), Arc::downgrade(&new_group));
        new_process
    }

    pub fn is_zombie(&self) -> bool {
        self.is_zombie.load(Ordering::Acquire)
    }

    fn set_zombie(&self) {
        self.is_zombie.store(true, Ordering::Release);
    }

    fn get_child_reaper(&self) -> Option<Arc<Process>> {
        // TODO: child reaper
        if self.pid == 1 {
            // TODO: check if the reaper is zombie
            return None;
        }
        PROCESS_TABLE.lock().get(&1).cloned()
    }

    pub(crate) fn exit(self: &Arc<Self>) {
        assert!(
            !self.is_zombie(),
            "[process] process {} is already exited",
            self.pid
        );
        self.set_zombie();
        // move children to reaper process
        let reaper = self.get_child_reaper();
        if let Some(reaper) = reaper {
            let mut children = self.children.lock();
            let mut reaper_children = reaper.children.lock();
            let weak_reaper = Arc::downgrade(&reaper);
            // `children` of the origin process will be cleared
            for (pid, child) in core::mem::take(&mut *children) {
                *child.parent.lock() = weak_reaper.clone();
                reaper_children.insert(pid, child);
            }
        }

        if self.get_parent().is_none() {
            // will become parentless zombie, release itself
            self.release();
        }
    }

    pub fn release(self: &Arc<Self>) {
        assert!(
            self.is_zombie(),
            "[process] process {} is not exited",
            self.pid
        );
        // TODO: remove threads
        // remove from parent
        if let Some(parent) = self.get_parent() {
            parent.children.lock().remove(&self.pid);
        }
        // remove from process group
        let group = self.get_group();
        group.remove_process(self.pid);
        // remove from process table
        PROCESS_TABLE.lock().remove(&self.pid);
    }

    pub(crate) fn add_thread(&self, thread: Arc<Thread>) {
        self.threads.lock().insert(thread.get_tid(), thread);
    }

    pub(crate) fn remove_thread(self: &Arc<Self>, tid: Pid, exit_code: i32) {
        self.threads.lock().remove(&tid);
        if self.threads.lock().is_empty() {
            self.exit_code.store(exit_code, Ordering::Relaxed);
            self.exit()
        }
    }

    pub fn create_thread(self: &Arc<Self>) -> Arc<Thread> {
        let tid = generate_next_pid();
        // `create_thread` will add the thread to the process
        create_thread(tid, Arc::downgrade(self))
    }

    pub fn get_main_thread(&self) -> Option<Arc<Thread>> {
        self.threads.lock().get(&self.pid).cloned()
    }

    pub fn get_threads(&self) -> Vec<Arc<Thread>> {
        self.threads.lock().values().cloned().collect()
    }

    pub fn get_exit_code(&self) -> i32 {
        assert!(self.is_zombie());
        self.exit_code.load(Ordering::Relaxed)
    }
}

static PROCESS_TABLE: Mutex<BTreeMap<Pid, Arc<Process>>> =
    Mutex::new(BTreeMap::<Pid, Arc<Process>>::new());

static NEXT_PID: AtomicU32 = AtomicU32::new(1);

fn generate_next_pid() -> Pid {
    NEXT_PID.fetch_add(1, Ordering::Acquire)
}

/// Create a new process if the process does not exist
fn create_process(pid: Pid, parent: Weak<Process>, group: Weak<ProcessGroup>) -> Arc<Process> {
    let mut process_table = PROCESS_TABLE.lock();
    if process_table.contains_key(&pid) {
        panic!("[process] process with id {} already exists", pid);
    }
    // create process
    let process = Process::new(pid, parent.clone(), group.clone());
    // add to process group
    let group = group.upgrade().unwrap();
    group.add_process(process.clone());
    // add to parent
    if let Some(parent) = parent.upgrade() {
        parent.children.lock().insert(pid, process.clone());
    }
    // create main thread
    create_thread(pid, Arc::downgrade(&process));
    // add to process table
    process_table.insert(pid, process.clone());
    process
}

pub fn get_process(pid: Pid) -> Option<Arc<Process>> {
    PROCESS_TABLE.lock().get(&pid).cloned()
}

pub fn get_all_processes() -> Vec<Arc<Process>> {
    PROCESS_TABLE.lock().values().cloned().collect()
}
