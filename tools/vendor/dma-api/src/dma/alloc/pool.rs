use core::ops::{Deref, DerefMut};

use alloc::{
    collections::VecDeque,
    sync::{Arc, Weak},
};
use spin::Mutex;

use crate::{DVec, Direction};

#[derive(Debug, Clone)]
pub struct DVecConfig {
    pub dma_mask: u64,
    pub align: usize,
    pub size: usize,
    pub direction: Direction,
}

#[derive(Clone)]
pub struct DVecPool {
    inner: Arc<Mutex<Inner>>,
}

pub struct DBuff {
    data: Option<DVec<u8>>,
    pool: Weak<Mutex<Inner>>,
}

unsafe impl Send for DBuff {}

impl Deref for DBuff {
    type Target = DVec<u8>;

    fn deref(&self) -> &Self::Target {
        self.data.as_ref().unwrap()
    }
}

impl DerefMut for DBuff {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data.as_mut().unwrap()
    }
}

impl Drop for DBuff {
    fn drop(&mut self) {
        if let Some(data) = self.data.take() {
            if let Some(pool) = self.pool.upgrade() {
                let mut inner = pool.lock();
                inner.dealloc(data);
            }
        }
    }
}

struct Inner {
    config: DVecConfig,
    pool: VecDeque<DVec<u8>>,
}

impl Inner {
    fn alloc(&mut self) -> Option<DVec<u8>> {
        self.pool.pop_front()
    }

    fn dealloc(&mut self, dvec: DVec<u8>) {
        self.pool.push_back(dvec);
    }
}

impl DVecPool {
    pub fn new_pool(config: DVecConfig, cap: usize) -> DVecPool {
        let mut pool = VecDeque::with_capacity(cap);
        for _ in 0..cap {
            if let Ok(dvec) =
                DVec::zeros(config.dma_mask, config.size, config.align, config.direction)
            {
                pool.push_back(dvec);
            }
        }

        DVecPool {
            inner: Arc::new(Mutex::new(Inner { pool, config })),
        }
    }

    pub fn alloc(&self) -> Result<DBuff, crate::dma::alloc::DError> {
        let config = {
            let mut inner = self.inner.lock();
            if let Some(dvec) = inner.alloc() {
                return Ok(DBuff {
                    data: Some(dvec),
                    pool: Arc::downgrade(&self.inner),
                });
            } else {
                inner.config.clone()
            }
        };

        let dvec = DVec::zeros(config.dma_mask, config.size, config.align, config.direction)?;
        Ok(DBuff {
            data: Some(dvec),
            pool: Arc::downgrade(&self.inner),
        })
    }
}
