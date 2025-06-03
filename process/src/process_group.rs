use crate::Pid;
use crate::process::Process;
use crate::session::Session;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use spin::Mutex;

pub struct ProcessGroup {
    pgid: Pid,
    processes: Mutex<BTreeMap<Pid, Arc<Process>>>,
    pub(crate) session: Weak<Session>,
}

impl ProcessGroup {
    /// Get process group id
    pub fn get_pgid(&self) -> Pid {
        self.pgid
    }

    pub fn get_session(&self) -> Arc<Session> {
        self.session.upgrade().unwrap()
    }

    /// Get the leader process of the process group
    /// Return `None` if the leader process does not exist (maybe exited)
    pub fn get_leader(&self) -> Option<Arc<Process>> {
        // "leader" process is the process with the same id as the process group id
        self.processes.lock().get(&self.pgid).cloned()
    }

    pub fn add_process(&self, process: Arc<Process>) {
        self.processes.lock().insert(process.get_pid(), process);
    }

    pub fn remove_process(&self, pid: Pid) {
        self.processes.lock().remove(&pid);
        if self.processes.lock().is_empty() {
            // if the process group is empty, remove it from the session
            self.get_session().remove_process_group(self.pgid);
            // remove from the process group table
            PROCESS_GROUP_TABLE.lock().remove(&self.pgid);
        }
    }

    pub fn get_processes(&self) -> Vec<Arc<Process>> {
        self.processes.lock().values().cloned().collect()
    }

    /// Create a new process group with the given process group id and session
    fn new(id: Pid, session: Weak<Session>) -> Arc<Self> {
        Arc::new(Self {
            pgid: id,
            processes: Mutex::new(BTreeMap::new()),
            session,
        })
    }
}

static PROCESS_GROUP_TABLE: Mutex<BTreeMap<Pid, Arc<ProcessGroup>>> =
    Mutex::new(BTreeMap::<Pid, Arc<ProcessGroup>>::new());

/// Create a new process group if the process group does not exist
pub(crate) fn create_process_group(pgid: Pid, session: Weak<Session>) -> Arc<ProcessGroup> {
    let mut process_group_table = PROCESS_GROUP_TABLE.lock();
    if process_group_table.contains_key(&pgid) {
        panic!("[process] process group with id {} already exists", pgid);
    }
    let process_group = ProcessGroup::new(pgid, session.clone());
    let session = session.upgrade().unwrap();
    session.add_process_group(pgid, process_group.clone());
    process_group_table.insert(pgid, process_group.clone());
    process_group
}

pub fn get_process_group(pgid: Pid) -> Option<Arc<ProcessGroup>> {
    let process_group_table = PROCESS_GROUP_TABLE.lock();
    process_group_table.get(&pgid).cloned()
}
