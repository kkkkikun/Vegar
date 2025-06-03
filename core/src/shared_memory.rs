use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use axalloc::global_allocator;
use axerrno::{LinuxError, LinuxResult};
use axsync::Mutex;
use core::sync::atomic::{AtomicU32, Ordering};
use memory_addr::PAGE_SIZE_4K;

pub struct SharedMemory {
    /// The key of the shared memory segment
    pub key: u32,
    /// Virtual kernel address of the shared memory segment
    pub addr: usize,
    /// Page count of the shared memory segment
    pub page_count: usize,
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        let allocator = global_allocator();
        allocator.dealloc_pages(self.addr, self.page_count);
        info!(
            "[SharedMemory] dealloc pages: addr: {:#x}, page_count: {}, key: {}",
            self.addr, self.page_count, self.key
        );
    }
}

pub struct SharedMemoryManager {
    mem_map: Mutex<BTreeMap<u32, Arc<SharedMemory>>>,
    next_key: AtomicU32,
}

impl SharedMemoryManager {
    pub const fn new() -> Self {
        SharedMemoryManager {
            mem_map: Mutex::new(BTreeMap::new()),
            next_key: AtomicU32::new(1),
        }
    }

    pub fn next_available_key(&self) -> u32 {
        let next_key = self.next_key.fetch_add(1, Ordering::Relaxed);
        self.mem_map.lock().keys().max().unwrap_or(&0) + next_key
    }

    pub fn get(&self, key: u32) -> Option<Arc<SharedMemory>> {
        self.mem_map.lock().get(&key).cloned()
    }

    pub fn create(&self, key: u32, size: usize) -> LinuxResult<Arc<SharedMemory>> {
        let page_count = size.div_ceil(PAGE_SIZE_4K);
        let allocator = global_allocator();
        // TODO: more error checking
        let vaddr = allocator
            .alloc_pages(page_count, PAGE_SIZE_4K)
            .map_err(|_| LinuxError::ENOMEM)?;
        let shared_memory = SharedMemory {
            key,
            addr: vaddr,
            page_count: size,
        };
        let shared_memory = Arc::new(shared_memory);
        self.mem_map.lock().insert(key, shared_memory.clone());
        Ok(shared_memory)
    }

    pub fn delete(&self, key: u32) -> bool {
        self.mem_map.lock().remove(&key).is_some()
    }
}

pub static SHARED_MEMORY_MANAGER: SharedMemoryManager = SharedMemoryManager::new();
