/*!
Simple examples demonstrating the usage of mbarrier.
*/

use mbarrier::*;

fn main() {
    // Basic barrier usage
    println!("Testing memory barriers...");

    // Read memory barrier
    rmb();
    println!("Read memory barrier executed");

    // Write memory barrier
    wmb();
    println!("Write memory barrier executed");

    // General memory barrier
    mb();
    println!("General memory barrier executed");

    // SMP barriers
    smp_rmb();
    smp_wmb();
    smp_mb();
    println!("SMP barriers executed");

    println!("All barriers executed successfully!");
}
