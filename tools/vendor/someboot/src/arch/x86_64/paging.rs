use page_table_generic::{PageTableEntry, TableGeneric};

#[derive(Debug, Clone, Copy)]
pub struct Table {}
impl TableGeneric for Table {
    type P = Pte;

    const PAGE_SIZE: usize = 4;

    const LEVEL_BITS: &[usize] = &[9, 9, 9, 9];

    const MAX_BLOCK_LEVEL: usize = 2;

    fn flush(vaddr: Option<page_table_generic::VirtAddr>) {
        todo!()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Pte(u64);

impl PageTableEntry for Pte {
    fn valid(&self) -> bool {
        todo!()
    }

    fn paddr(&self) -> page_table_generic::PhysAddr {
        todo!()
    }

    fn set_paddr(&mut self, paddr: page_table_generic::PhysAddr) {
        todo!()
    }

    fn set_valid(&mut self, valid: bool) {
        todo!()
    }

    fn is_huge(&self) -> bool {
        todo!()
    }

    fn set_is_huge(&mut self, b: bool) {
        todo!()
    }
}
