use crate::resource::ResourceLimits;
use crate::shared_memory::SharedMemory;
use crate::task::WaitQueueWrapper;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use axmm::{AddrSpace, kernel_aspace};
use axns::AxNamespace;
use axsignal::Signo;
use axsignal::api::{ProcessSignalManager, SignalActions, ThreadSignalManager};
use axsync::RawMutex;
use axtask::WaitQueue;
use core::sync::atomic::{AtomicUsize, Ordering};
use memory_addr::{VirtAddr, VirtAddrRange};
use spin::Mutex;
use undefined_process::Pid;

pub struct ProcessData {
    /// The command line arguments
    pub command_line: Mutex<Vec<String>>,

    // address space related are shared with all threads
    /// The virtual memory address space.
    pub addr_space: Arc<Mutex<AddrSpace>>,
    /// The user heap bottom
    heap_bottom: AtomicUsize,
    /// The user heap top
    heap_top: AtomicUsize,
    /// resource limits
    pub resource_limits: Arc<Mutex<ResourceLimits>>,
    /// The child exit wait queue
    pub child_exit_wq: WaitQueue,
    /// The exit signal of the thread
    pub exit_signal: Option<Signo>,
    /// The process signal manager
    pub signal: Arc<ProcessSignalManager<RawMutex, WaitQueueWrapper>>,
    /// The futex table
    pub futex_table: Mutex<BTreeMap<usize, Arc<WaitQueue>>>,
    /// Shared memory
    pub shared_memory: Mutex<BTreeMap<VirtAddr, Arc<SharedMemory>>>,
}

impl ProcessData {
    pub fn new(
        command_line: Vec<String>,
        addr_space: Arc<Mutex<AddrSpace>>,
        signal_actions: Arc<axsync::Mutex<SignalActions>>,
        exit_signal: Option<Signo>,
    ) -> Self {
        Self {
            command_line: Mutex::new(command_line),
            addr_space,
            heap_bottom: AtomicUsize::new(axconfig::plat::USER_HEAP_BASE),
            heap_top: AtomicUsize::new(axconfig::plat::USER_HEAP_BASE),
            resource_limits: Arc::new(Mutex::new(ResourceLimits::new())),
            futex_table: Mutex::new(BTreeMap::new()),
            child_exit_wq: WaitQueue::new(),
            exit_signal,
            signal: Arc::new(ProcessSignalManager::new(
                signal_actions,
                axconfig::plat::SIGNAL_TRAMPOLINE,
            )),
            shared_memory: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn get_heap_bottom(&self) -> usize {
        self.heap_bottom.load(Ordering::Acquire)
    }

    pub fn set_heap_bottom(&self, bottom: usize) {
        self.heap_bottom.store(bottom, Ordering::Release)
    }

    pub fn get_heap_top(&self) -> usize {
        self.heap_top.load(Ordering::Acquire)
    }

    pub fn set_heap_top(&self, top: usize) {
        self.heap_top.store(top, Ordering::Release)
    }

    /// Linux manual: A "clone" child is one which delivers no signal, or a
    /// signal other than SIGCHLD to its parent upon termination.
    pub fn is_clone_child(&self) -> bool {
        self.exit_signal != Some(Signo::SIGCHLD)
    }
}

impl Drop for ProcessData {
    fn drop(&mut self) {
        trace!("process data drop: process={:?}", self.command_line.lock());
        // TODO: prevent memory leak
        if !cfg!(target_arch = "aarch64") && !cfg!(target_arch = "loongarch64") {
            // See [`crate::new_user_aspace`]
            let kernel = kernel_aspace().lock();
            self.addr_space
                .lock()
                .clear_mappings(VirtAddrRange::from_start_size(kernel.base(), kernel.size()));
        }
    }
}

pub struct ThreadData {
    /// only for TABLE management
    tid: Pid,
    /// The process data
    pub process_data: Arc<ProcessData>,
    /// The resource namespace, used by FD_TABLE and CURRENT_DIR, etc.
    pub namespace: AxNamespace,
    /// The clear thread tid field
    ///
    /// See <https://manpages.debian.org/unstable/manpages-dev/set_tid_address.2.en.html#clear_child_tid>
    ///
    /// When the thread exits, the kernel clears the word at this address if it is not NULL.
    pub addr_clear_child_tid: AtomicUsize,
    /// The set thread tid field
    pub addr_set_child_tid: AtomicUsize,
    /// The thread-level signal manager
    pub signal: ThreadSignalManager<RawMutex, WaitQueueWrapper>,
}

impl ThreadData {
    fn new(process_data: Arc<ProcessData>, tid: Pid) -> Self {
        Self {
            namespace: AxNamespace::new_thread_local(),
            addr_clear_child_tid: AtomicUsize::new(0),
            addr_set_child_tid: AtomicUsize::new(0),
            signal: ThreadSignalManager::new(process_data.signal.clone()),
            process_data,
            tid,
        }
    }
}

impl Drop for ThreadData {
    fn drop(&mut self) {
        // remove form the thread data table
        trace!("thread data drop: tid={}", self.tid);
        assert!(!THREAD_DATA_TABLE.lock().remove(&self.tid).is_none())
    }
}

static THREAD_DATA_TABLE: Mutex<BTreeMap<Pid, Weak<ThreadData>>> = Mutex::new(BTreeMap::new());

pub fn create_thread_data(process_data: Arc<ProcessData>, tid: Pid) -> Arc<ThreadData> {
    let thread_data = Arc::new(ThreadData::new(process_data, tid));
    let mut thread_data_table = THREAD_DATA_TABLE.lock();
    thread_data_table.insert(tid, Arc::downgrade(&thread_data));
    thread_data
}

pub fn get_thread_data(tid: Pid) -> Option<Arc<ThreadData>> {
    let thread_data_table = THREAD_DATA_TABLE.lock();
    let weak_thread_data = thread_data_table.get(&tid)?;
    assert!(
        weak_thread_data.strong_count() > 0,
        "Thread data is not alive"
    );
    weak_thread_data.upgrade()
}

pub fn get_process_data(pid: Pid) -> Option<Arc<ProcessData>> {
    let thread_data = get_thread_data(pid)?;
    Some(thread_data.process_data.clone())
}
