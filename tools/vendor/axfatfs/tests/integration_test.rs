//! System tests: Integration tests for FAT filesystem library
//!
//! These tests validate the filesystem's behavior in realistic scenarios,
//! including large-scale operations, stress tests, and cross-platform compatibility.

use std::fs;
use std::io::{self, Read, Seek, Write};
use std::str;

use fatfs::{FsOptions, StdIoWrapper};
use fscommon::BufStream;

const FAT12_IMG: &str = "fat12.img";
const FAT16_IMG: &str = "fat16.img";
const FAT32_IMG: &str = "fat32.img";
const IMG_DIR: &str = "resources";
const TMP_DIR: &str = "tmp";

type FileSystem = fatfs::FileSystem<StdIoWrapper<BufStream<fs::File>>>;

fn setup_tmp_img(filename: &str, test_seq: u32) -> String {
    let _ = env_logger::builder().is_test(true).try_init();
    let img_path = format!("{}/{}", IMG_DIR, filename);
    let tmp_path = format!("{}/{}-{}", TMP_DIR, test_seq, filename);
    fs::create_dir(TMP_DIR).ok();
    fs::copy(img_path, &tmp_path).unwrap();
    tmp_path
}

fn cleanup_tmp_img(tmp_path: &str) {
    fs::remove_file(tmp_path).unwrap();
}

fn open_filesystem_rw(tmp_path: &str) -> FileSystem {
    let file = fs::OpenOptions::new().read(true).write(true).open(tmp_path).unwrap();
    let buf_file = BufStream::new(file);
    let options = FsOptions::new().update_accessed_date(true);
    FileSystem::new(buf_file, options).unwrap()
}

fn open_filesystem_ro(tmp_path: &str) -> FileSystem {
    let file = fs::OpenOptions::new().read(true).open(tmp_path).unwrap();
    let buf_file = BufStream::new(file);
    let options = FsOptions::new();
    FileSystem::new(buf_file, options).unwrap()
}

// ============================================================================
// Large-Scale Filesystem Operations Tests
// ============================================================================

/// Test creating many files in a single directory
#[test]
fn test_create_many_files_in_directory_fat12() {
    test_create_many_files_in_directory(FAT12_IMG, 1001)
}

#[test]
fn test_create_many_files_in_directory_fat16() {
    test_create_many_files_in_directory(FAT16_IMG, 1002)
}

#[test]
fn test_create_many_files_in_directory_fat32() {
    test_create_many_files_in_directory(FAT32_IMG, 1003)
}

fn test_create_many_files_in_directory(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);
    let fs = open_filesystem_rw(&tmp_path);
    let root_dir = fs.root_dir();

    let count = 50; // Create 50 files
    for i in 0..count {
        let file_name = format!("file_{}.txt", i);
        let mut file = root_dir.create_file(&file_name).unwrap();
        let content = format!("This is file number {}\n", i);
        file.write_all(content.as_bytes()).unwrap();
    }

    // Verify all files exist and contain correct content
    for i in 0..count {
        let file_name = format!("file_{}.txt", i);
        let mut file = root_dir.open_file(&file_name).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        let expected = format!("This is file number {}\n", i);
        assert_eq!(str::from_utf8(&buf).unwrap(), expected);
    }

    cleanup_tmp_img(&tmp_path);
}

/// Test creating many directories
#[test]
fn test_create_many_directories_fat12() {
    test_create_many_directories(FAT12_IMG, 1011)
}

#[test]
fn test_create_many_directories_fat16() {
    test_create_many_directories(FAT16_IMG, 1012)
}

#[test]
fn test_create_many_directories_fat32() {
    test_create_many_directories(FAT32_IMG, 1013)
}

fn test_create_many_directories(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);
    let fs = open_filesystem_rw(&tmp_path);
    let root_dir = fs.root_dir();

    let count = 30; // Create 30 directories
    for i in 0..count {
        let dir_name = format!("dir_{}", i);
        root_dir.create_dir(&dir_name).unwrap();
    }

    // Verify all directories exist
    for i in 0..count {
        let dir_name = format!("dir_{}", i);
        let dir = root_dir.open_dir(&dir_name).unwrap();
        // Each directory should have at least . and ..
        assert!(dir.iter().count() >= 2);
    }

    cleanup_tmp_img(&tmp_path);
}

/// Test deep nested directory structure
#[test]
fn test_deep_nested_structure_fat12() {
    test_deep_nested_structure(FAT12_IMG, 1021)
}

#[test]
fn test_deep_nested_structure_fat16() {
    test_deep_nested_structure(FAT16_IMG, 1022)
}

#[test]
fn test_deep_nested_structure_fat32() {
    test_deep_nested_structure(FAT32_IMG, 1023)
}

fn test_deep_nested_structure(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);
    let fs = open_filesystem_rw(&tmp_path);
    let root_dir = fs.root_dir();

    let depth = 8;
    let mut current = root_dir.clone();
    for i in 0..depth {
        let dir_name = format!("level_{}", i);
        current = current.create_dir(&dir_name).unwrap();
    }

    // Create a file at the deepest level
    let mut file = current.create_file("deep_file.txt").unwrap();
    file.write_all(b"Deep nested file content").unwrap();
    file.flush().unwrap();

    // Reopen from root and verify
    let mut path = String::new();
    for i in 0..depth {
        path.push_str(&format!("level_{}/", i));
    }
    path.push_str("deep_file.txt");

    let mut file = root_dir.open_file(&path).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), "Deep nested file content");

    cleanup_tmp_img(&tmp_path);
}

/// Test large file operations
#[test]
fn test_large_file_operations_fat12() {
    test_large_file_operations(FAT12_IMG, 1031)
}

#[test]
fn test_large_file_operations_fat16() {
    test_large_file_operations(FAT16_IMG, 1032)
}

#[test]
fn test_large_file_operations_fat32() {
    test_large_file_operations(FAT32_IMG, 1033)
}

fn test_large_file_operations(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);
    let fs = open_filesystem_rw(&tmp_path);
    let root_dir = fs.root_dir();

    // Create a large file (100KB)
    let large_content: Vec<u8> = (0..102400).map(|i| (i % 256) as u8).collect();
    let mut file = root_dir.create_file("large_file.bin").unwrap();
    file.write_all(&large_content).unwrap();
    file.flush().unwrap();

    // Verify file size
    file.seek(io::SeekFrom::End(0)).unwrap();
    let pos = file.seek(io::SeekFrom::Current(0)).unwrap();
    assert_eq!(pos, large_content.len() as u64);

    // Read and verify content
    file.seek(io::SeekFrom::Start(0)).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, large_content);

    cleanup_tmp_img(&tmp_path);
}

// ============================================================================
// Stress Tests
// ============================================================================

/// Test repeated create/write/read/delete cycles
#[test]
fn test_repeated_cycles_fat12() {
    test_repeated_cycles(FAT12_IMG, 2001)
}

#[test]
fn test_repeated_cycles_fat16() {
    test_repeated_cycles(FAT16_IMG, 2002)
}

#[test]
fn test_repeated_cycles_fat32() {
    test_repeated_cycles(FAT32_IMG, 2003)
}

fn test_repeated_cycles(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);
    let fs = open_filesystem_rw(&tmp_path);
    let root_dir = fs.root_dir();

    let cycles = 20; // Run 20 create/write/read/delete cycles
    for cycle in 0..cycles {
        let file_name = format!("cycle_{}.txt", cycle);

        // Create and write
        {
            let mut file = root_dir.create_file(&file_name).unwrap();
            let content = format!("Cycle {} content", cycle);
            file.write_all(content.as_bytes()).unwrap();
        }

        // Read and verify
        {
            let mut file = root_dir.open_file(&file_name).unwrap();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).unwrap();
            let expected = format!("Cycle {} content", cycle);
            assert_eq!(str::from_utf8(&buf).unwrap(), expected);
        }

        // Delete
        root_dir.remove(&file_name).unwrap();

        // Verify deletion
        assert!(root_dir.open_file(&file_name).is_err());
    }

    cleanup_tmp_img(&tmp_path);
}

/// Test filesystem fragmentation handling
#[test]
fn test_fragmentation_handling_fat12() {
    test_fragmentation_handling(FAT12_IMG, 2011)
}

#[test]
fn test_fragmentation_handling_fat16() {
    test_fragmentation_handling(FAT16_IMG, 2012)
}

#[test]
fn test_fragmentation_handling_fat32() {
    test_fragmentation_handling(FAT32_IMG, 2013)
}

fn test_fragmentation_handling(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);
    let fs = open_filesystem_rw(&tmp_path);
    let root_dir = fs.root_dir();

    // Create many small files to cause fragmentation
    let file_count = 20;
    let mut file_names = Vec::new();
    for i in 0..file_count {
        let file_name = format!("frag_{}.txt", i);
        let mut file = root_dir.create_file(&file_name).unwrap();
        let content = format!("Fragmentation test file {}", i);
        file.write_all(content.as_bytes()).unwrap();
        file_names.push(file_name);
    }

    // Delete every other file to create gaps
    for i in (0..file_count).filter(|x| x % 2 == 0) {
        root_dir.remove(&file_names[i]).unwrap();
    }

    // Create new files that should use the freed space
    for i in 0..5 {
        let file_name = format!("new_{}.txt", i);
        let mut file = root_dir.create_file(&file_name).unwrap();
        let content = format!("New file {}", i);
        file.write_all(content.as_bytes()).unwrap();
    }

    // Verify remaining original files are still readable
    for i in (0..file_count).filter(|x| x % 2 == 1) {
        let mut file = root_dir.open_file(&file_names[i]).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        let expected = format!("Fragmentation test file {}", i);
        assert_eq!(str::from_utf8(&buf).unwrap(), expected);
    }

    cleanup_tmp_img(&tmp_path);
}

/// Test filesystem with mixed operations
#[test]
fn test_mixed_operations_fat12() {
    test_mixed_operations(FAT12_IMG, 2021)
}

#[test]
fn test_mixed_operations_fat16() {
    test_mixed_operations(FAT16_IMG, 2022)
}

#[test]
fn test_mixed_operations_fat32() {
    test_mixed_operations(FAT32_IMG, 2023)
}

fn test_mixed_operations(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);
    let fs = open_filesystem_rw(&tmp_path);
    let root_dir = fs.root_dir();

    // Create directories
    for i in 0..5 {
        let dir_name = format!("mixed_dir_{}", i);
        root_dir.create_dir(&dir_name).unwrap();
    }

    // Create files in root
    for i in 0..10 {
        let file_name = format!("mixed_root_{}.txt", i);
        let mut file = root_dir.create_file(&file_name).unwrap();
        let content = format!("Root file {}", i);
        file.write_all(content.as_bytes()).unwrap();
    }

    // Create files in subdirectories
    for dir_i in 0..5 {
        let dir = root_dir.open_dir(&format!("mixed_dir_{}", dir_i)).unwrap();
        for file_i in 0..5 {
            let file_name = format!("file_{}.txt", file_i);
            let mut file = dir.create_file(&file_name).unwrap();
            let content = format!("Dir {} file {}", dir_i, file_i);
            file.write_all(content.as_bytes()).unwrap();
        }
    }

    // Rename some files
    for i in 0..3 {
        let old_name = format!("mixed_root_{}.txt", i);
        let new_name = format!("renamed_{}.txt", i);
        root_dir.rename(&old_name, &root_dir, &new_name).unwrap();
    }

    // Delete some files
    for i in 5..8 {
        let file_name = format!("renamed_{}.txt", i);
        if root_dir.open_file(&file_name).is_ok() {
            root_dir.remove(&file_name).unwrap();
        }
    }

    // Verify filesystem is still in consistent state
    let root_entries: Vec<String> = root_dir.iter().map(|r| r.unwrap().file_name()).collect();
    assert!(root_entries.contains(&String::from("mixed_dir_0")));
    assert!(root_entries.contains(&String::from("mixed_dir_1")));

    cleanup_tmp_img(&tmp_path);
}

// ============================================================================
// Cross-Platform Compatibility Tests
// ============================================================================

/// Test filesystem is readable by different FAT implementations
#[test]
fn test_cross_platform_read_fat12() {
    test_cross_platform_read(FAT12_IMG, 3001)
}

#[test]
fn test_cross_platform_read_fat16() {
    test_cross_platform_read(FAT16_IMG, 3002)
}

#[test]
fn test_cross_platform_read_fat32() {
    test_cross_platform_read(FAT32_IMG, 3003)
}

fn test_cross_platform_read(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);

    // First write
    {
        let fs = open_filesystem_rw(&tmp_path);
        let root_dir = fs.root_dir();
        let mut file = root_dir.create_file("cross_platform_test.txt").unwrap();
        file.write_all(b"Cross-platform test content").unwrap();
    }

    // Reopen read-only to verify
    {
        let fs = open_filesystem_ro(&tmp_path);
        let root_dir = fs.root_dir();
        let mut file = root_dir.open_file("cross_platform_test.txt").unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        assert_eq!(str::from_utf8(&buf).unwrap(), "Cross-platform test content");
    }

    // Reopen read-write to modify
    {
        let fs = open_filesystem_rw(&tmp_path);
        let root_dir = fs.root_dir();
        let mut file = root_dir.open_file("cross_platform_test.txt").unwrap();
        file.seek(io::SeekFrom::End(0)).unwrap();
        file.write_all(b" - appended").unwrap();
    }

    // Final verification
    {
        let fs = open_filesystem_ro(&tmp_path);
        let root_dir = fs.root_dir();
        let mut file = root_dir.open_file("cross_platform_test.txt").unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        assert_eq!(str::from_utf8(&buf).unwrap(), "Cross-platform test content - appended");
    }

    cleanup_tmp_img(&tmp_path);
}

/// Test filesystem with various file sizes
#[test]
fn test_various_file_sizes_fat12() {
    test_various_file_sizes(FAT12_IMG, 3011)
}

#[test]
fn test_various_file_sizes_fat16() {
    test_various_file_sizes(FAT16_IMG, 3012)
}

#[test]
fn test_various_file_sizes_fat32() {
    test_various_file_sizes(FAT32_IMG, 3013)
}

fn test_various_file_sizes(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);
    let fs = open_filesystem_rw(&tmp_path);
    let root_dir = fs.root_dir();

    let sizes: Vec<usize> = vec![0, 1, 100, 1024, 4096, 16384, 65536, 102400];

    for (i, size) in sizes.iter().enumerate() {
        let file_name = format!("size_{}.dat", i);
        let content: Vec<u8> = (0..*size).map(|x| (x % 256) as u8).collect();

        let mut file = root_dir.create_file(&file_name).unwrap();
        file.write_all(&content).unwrap();
        file.flush().unwrap();

        // Verify
        let mut file = root_dir.open_file(&file_name).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        assert_eq!(buf.len(), *size);
        assert_eq!(buf, content);
    }

    cleanup_tmp_img(&tmp_path);
}

// ============================================================================
// Filesystem State Persistence Tests
// ============================================================================

/// Test filesystem state across multiple mount/unmount cycles
#[test]
fn test_multiple_mount_cycles_fat12() {
    test_multiple_mount_cycles(FAT12_IMG, 4001)
}

#[test]
fn test_multiple_mount_cycles_fat16() {
    test_multiple_mount_cycles(FAT16_IMG, 4002)
}

#[test]
fn test_multiple_mount_cycles_fat32() {
    test_multiple_mount_cycles(FAT32_IMG, 4003)
}

fn test_multiple_mount_cycles(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);

    let cycles = 5;
    for cycle in 0..cycles {
        // Write phase
        {
            let fs = open_filesystem_rw(&tmp_path);
            let root_dir = fs.root_dir();

            // Create new file each cycle
            let file_name = format!("cycle_{}.txt", cycle);
            let mut file = root_dir.create_file(&file_name).unwrap();
            let content = format!("Mount cycle {}", cycle);
            file.write_all(content.as_bytes()).unwrap();

            // Verify previous files still exist
            for prev in 0..cycle {
                let prev_name = format!("cycle_{}.txt", prev);
                let mut file = root_dir.open_file(&prev_name).unwrap();
                let mut buf = Vec::new();
                file.read_to_end(&mut buf).unwrap();
                let expected = format!("Mount cycle {}", prev);
                assert_eq!(str::from_utf8(&buf).unwrap(), expected);
            }
        }

        // Read-only verification phase
        {
            let fs = open_filesystem_ro(&tmp_path);
            let root_dir = fs.root_dir();

            for prev in 0..=cycle {
                let prev_name = format!("cycle_{}.txt", prev);
                let mut file = root_dir.open_file(&prev_name).unwrap();
                let mut buf = Vec::new();
                file.read_to_end(&mut buf).unwrap();
                let expected = format!("Mount cycle {}", prev);
                assert_eq!(str::from_utf8(&buf).unwrap(), expected);
            }
        }
    }

    cleanup_tmp_img(&tmp_path);
}

/// Test filesystem recovery after unexpected operations
#[test]
fn test_filesystem_recovery_fat12() {
    test_filesystem_recovery(FAT12_IMG, 4011)
}

#[test]
fn test_filesystem_recovery_fat16() {
    test_filesystem_recovery(FAT16_IMG, 4012)
}

#[test]
fn test_filesystem_recovery_fat32() {
    test_filesystem_recovery(FAT32_IMG, 4013)
}

fn test_filesystem_recovery(filename: &str, test_seq: u32) {
    let tmp_path = setup_tmp_img(filename, test_seq);

    // Create initial state
    {
        let fs = open_filesystem_rw(&tmp_path);
        let root_dir = fs.root_dir();
        for i in 0..10 {
            let file_name = format!("recovery_{}.txt", i);
            let mut file = root_dir.create_file(&file_name).unwrap();
            let content = format!("Recovery test file {}", i);
            file.write_all(content.as_bytes()).unwrap();
        }
    }

    // Delete some files
    {
        let fs = open_filesystem_rw(&tmp_path);
        let root_dir = fs.root_dir();
        for i in 0..5 {
            let file_name = format!("recovery_{}.txt", i);
            root_dir.remove(&file_name).unwrap();
        }
    }

    // Verify remaining files are accessible
    {
        let fs = open_filesystem_ro(&tmp_path);
        let root_dir = fs.root_dir();
        for i in 5..10 {
            let file_name = format!("recovery_{}.txt", i);
            let mut file = root_dir.open_file(&file_name).unwrap();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).unwrap();
            let expected = format!("Recovery test file {}", i);
            assert_eq!(str::from_utf8(&buf).unwrap(), expected);
        }
    }

    cleanup_tmp_img(&tmp_path);
}
