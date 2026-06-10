/*!
Performance comparison example.

This example compares the performance of different barrier types
and demonstrates the overhead they introduce.
*/

use std::time::Instant;

use mbarrier::*;

const ITERATIONS: usize = 1_000_000;

fn benchmark_barrier<F: Fn()>(name: &str, barrier_fn: F) {
    let start = Instant::now();
    // Simple benchmark that just runs the barriers
    // In a no_std environment, we can't measure time precisely
    for _ in 0..ITERATIONS {
        barrier_fn();
    }
    let duration = start.elapsed();
    println!("{}: {} ms", name, duration.as_millis());
}

fn main() {
    // Benchmark no-op (baseline)
    benchmark_barrier("No-op baseline", || {});

    // Benchmark read barriers
    benchmark_barrier("rmb()", rmb);
    benchmark_barrier("smp_rmb()", smp_rmb);

    // Benchmark write barriers
    benchmark_barrier("wmb()", wmb);
    benchmark_barrier("smp_wmb()", smp_wmb);

    // Benchmark general barriers
    benchmark_barrier("mb()", mb);
    benchmark_barrier("smp_mb()", smp_mb);

    // Benchmark completed successfully if we reach here
}
