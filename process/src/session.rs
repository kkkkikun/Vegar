use crate::Pid;
use crate::process::Process;
use crate::process_group::ProcessGroup;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use spin::Mutex;

pub struct Session {
    sid: Pid,
    process_groups: Mutex<BTreeMap<Pid, Arc<ProcessGroup>>>,
}

impl Session {
    /// Get session id
    pub fn get_sid(&self) -> Pid {
        self.sid
    }

    pub fn get_process_group(&self, pgid: Pid) -> Option<Arc<ProcessGroup>> {
        self.process_groups.lock().get(&pgid).cloned()
    }

    /// Get the leader process of the session
    /// Return `None` if the leader process does not exist (maybe exited)
    /// TODO
    pub fn get_leader(&self) -> Option<Arc<Process>> {
        // "leader" process is the process with the same id as the session id
        let process_group = self.process_groups.lock().get(&self.sid).cloned();
        process_group.and_then(|pg| pg.get_leader())
    }

    pub fn add_process_group(&self, pgid: Pid, process_group: Arc<ProcessGroup>) {
        self.process_groups.lock().insert(pgid, process_group);
    }

    pub fn remove_process_group(&self, pgid: Pid) {
        self.process_groups.lock().remove(&pgid);
        if self.process_groups.lock().is_empty() {
            // if the session is empty, remove it from the session table
            SESSION_TABLE.lock().remove(&self.sid);
        }
    }

    /// Create a new session with the given session id
    fn new(session_id: Pid) -> Arc<Self> {
        Arc::new(Self {
            sid: session_id,
            process_groups: Mutex::new(BTreeMap::new()),
        })
    }
}

static SESSION_TABLE: Mutex<BTreeMap<Pid, Arc<Session>>> =
    Mutex::new(BTreeMap::<Pid, Arc<Session>>::new());

/// Create a new session if the session does not exist
pub(crate) fn create_session(session_id: Pid) -> Arc<Session> {
    let mut session_table = SESSION_TABLE.lock();
    if session_table.contains_key(&session_id) {
        panic!("[process] session with id {} already exists", session_id);
    }
    let session = Session::new(session_id);
    session_table.insert(session_id, session.clone());
    session
}

pub fn get_session(session_id: Pid) -> Option<Arc<Session>> {
    SESSION_TABLE.lock().get(&session_id).cloned()
}
