# DMA API

[![Rust](https://github.com/drivercraft/dma-api/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/drivercraft/dma-api/actions/workflows/rust.yml)

## Example

```rust
use dma_api::*;

// ----- OS Side -----

init(&Impled);

struct Impled;

impl Osal for Impled {

    fn map(&self, addr: std::ptr::NonNull<u8>, size: usize, direction: Direction) -> u64 {
        /// your virtual to physical mapping code here.
        println!("map @{:?}, size {size:#x}, {direction:?}", addr);
        addr.as_ptr() as usize as _
    }

    fn unmap(&self, addr: std::ptr::NonNull<u8>, size: usize) {
        println!("unmap @{:?}, size {size:#x}", addr);
    }

    fn flush(&self, addr: std::ptr::NonNull<u8>, size: usize) {
        /// flush cache to memory
        println!("flush @{:?}, size {size:#x}", addr);
    }

    fn invalidate(&self, addr: std::ptr::NonNull<u8>, size: usize) {
        /// invalidate cache
        println!("invalidate @{:?}, size {size:#x}", addr);
    }
}

// then you can do some thing with the driver.

// ----- Driver Side -----

// use global allocator to alloc `to device` type memory
#[cfg(feature = "alloc")]
{
    let dma_mask = u64::MAX;

    let mut dma: DVec<u32> = DVec::zeros(dma_mask, 10, 0x1000, Direction::ToDevice).unwrap();
    // flush cache to memory.
    dma.set(0, 1);

    // do nothing with cache
    let o = dma.get(0).unwrap();

    assert_eq!(o, 1);
}
```
