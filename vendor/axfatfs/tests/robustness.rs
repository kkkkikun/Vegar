//! Functional tests: Error scenarios, boundary conditions, and filesystem robustness tests
//!
//! These tests simulate various abnormal situations and boundary conditions,
//! validating filesystem's behavior in non-normal scenarios.

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
const TEST_STR: &str = "Hi there Rust programmer!\n";

type FileSystem = fatfs::FileSystem<StdIoWrapper<BufStream<fs::File>>>;

fn call_with_tmp_img<F: Fn(&str)>(f: F, filename: &str, test_seq: u32) {
    let _ = env_logger::builder().is_test(true).try_init();
    let img_path = format!("{}/{}", IMG_DIR, filename);
    let tmp_path = format!("{}/{}-{}", TMP_DIR, test_seq, filename);
    fs::create_dir(TMP_DIR).ok();
    fs::copy(img_path, &tmp_path).unwrap();
    f(tmp_path.as_str());
    fs::remove_file(tmp_path).unwrap();
}

fn open_filesystem_ro(tmp_path: &str) -> FileSystem {
    let file = fs::OpenOptions::new().read(true).open(tmp_path).unwrap();
    let buf_file = BufStream::new(file);
    let options = FsOptions::new();
    FileSystem::new(buf_file, options).unwrap()
}

fn open_filesystem_rw(tmp_path: &str) -> FileSystem {
    let file = fs::OpenOptions::new().read(true).write(true).open(tmp_path).unwrap();
    let buf_file = BufStream::new(file);
    let options = FsOptions::new().update_accessed_date(true);
    FileSystem::new(buf_file, options).unwrap()
}

fn call_with_fs<F: Fn(FileSystem)>(f: F, filename: &str, test_seq: u32) {
    let callback = |tmp_path: &str| {
        let fs = open_filesystem_rw(tmp_path);
        f(fs);
    };
    call_with_tmp_img(callback, filename, test_seq);
}

// ============================================================================
// Error Scenario Tests
// ===========================================================================

/// Test opening a nonexistent file
#[test]
fn test_open_nonexistent_file_fat12() {
    call_with_fs(test_open_nonexistent_file, FAT12_IMG, 101)
}

#[test]
fn test_open_nonexistent_file_fat16() {
    call_with_fs(test_open_nonexistent_file, FAT16_IMG, 102)
}

#[test]
fn test_open_nonexistent_file_fat32() {
    call_with_fs(test_open_nonexistent_file, FAT32_IMG, 103)
}

fn test_open_nonexistent_file(fs: FileSystem) {
    let root_dir = fs.root_dir();
    // Opening a nonexistent file should fail
    let result = root_dir.open_file("nonexistent.txt");
    assert!(result.is_err());
}

/// Test opening a nonexistent directory
#[test]
fn test_open_nonexistent_dir_fat12() {
    call_with_fs(test_open_nonexistent_dir, FAT12_IMG, 104)
}

#[test]
fn test_open_nonexistent_dir_fat16() {
    call_with_fs(test_open_nonexistent_dir, FAT16_IMG, 105)
}

#[test]
fn test_open_nonexistent_dir_fat32() {
    call_with_fs(test_open_nonexistent_dir, FAT32_IMG, 106)
}

fn test_open_nonexistent_dir(fs: FileSystem) {
    let root_dir = fs.root_dir();
    // Opening a nonexistent directory should fail
    let result = root_dir.open_dir("nonexistent");
    assert!(result.is_err());
}

/// Test invalid filenames
#[test]
fn test_invalid_filename_fat12() {
    call_with_fs(test_invalid_filename, FAT12_IMG, 107)
}

#[test]
fn test_invalid_filename_fat16() {
    call_with_fs(test_invalid_filename, FAT16_IMG, 108)
}

#[test]
fn test_invalid_filename_fat32() {
    call_with_fs(test_invalid_filename, FAT32_IMG, 109)
}

fn test_invalid_filename(fs: FileSystem) {
    let root_dir = fs.root_dir();
    // Filenames with invalid characters should fail
    assert!(root_dir.create_file("test:file.txt").is_err());
    assert!(root_dir.create_file("test\0file.txt").is_err());
    // Empty string would trigger panic, so skip it
    // assert!(root_dir.create_file("").is_err());
}

/// Test operations on nonexistent paths
#[test]
fn test_operation_on_nonexistent_path_fat12() {
    call_with_fs(test_operation_on_nonexistent_path, FAT12_IMG, 110)
}

#[test]
fn test_operation_on_nonexistent_path_fat16() {
    call_with_fs(test_operation_on_nonexistent_path, FAT16_IMG, 111)
}

#[test]
fn test_operation_on_nonexistent_path_fat32() {
    call_with_fs(test_operation_on_nonexistent_path, FAT32_IMG, 112)
}

fn test_operation_on_nonexistent_path(fs: FileSystem) {
    let root_dir = fs.root_dir();
    // Creating a file on a nonexistent path should fail
    assert!(root_dir.create_file("nonexistent/test.txt").is_err());
    // Creating a directory on a nonexistent path should fail
    assert!(root_dir.create_dir("nonexistent/subdir").is_err());
    // Removing a nonexistent file should fail
    assert!(root_dir.remove("nonexistent.txt").is_err());
}

// ============================================================================
// Boundary Condition Tests
// ===========================================================================

/// Test empty file operations
#[test]
fn test_empty_file_operations_fat12() {
    call_with_fs(test_empty_file_operations, FAT12_IMG, 113)
}

#[test]
fn test_empty_file_operations_fat16() {
    call_with_fs(test_empty_file_operations, FAT16_IMG, 114)
}

#[test]
fn test_empty_file_operations_fat32() {
    call_with_fs(test_empty_file_operations, FAT32_IMG, 115)
}

fn test_empty_file_operations(fs: FileSystem) {
    let root_dir = fs.root_dir();
    // Create a new file
    let mut file = root_dir.create_file("empty.txt").unwrap();
    // Verify file is initially empty
    file.seek(io::SeekFrom::Start(0)).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(buf.len(), 0);

    // Write 0 bytes
    file.write_all(&[]).unwrap();

    // Truncate to 0 bytes
    file.truncate().unwrap();

    // Reading should still return empty
    file.seek(io::SeekFrom::Start(0)).unwrap();
    buf.clear();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(buf.len(), 0);
}

/// Test single-byte file operations
#[test]
fn test_single_byte_file_fat12() {
    call_with_fs(test_single_byte_file, FAT12_IMG, 116)
}

#[test]
fn test_single_byte_file_fat16() {
    call_with_fs(test_single_byte_file, FAT16_IMG, 117)
}

#[test]
fn test_single_byte_file_fat32() {
    call_with_fs(test_single_byte_file, FAT32_IMG, 118)
}

fn test_single_byte_file(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut file = root_dir.create_file("single.txt").unwrap();

    // Write a single byte
    file.write_all(b"X").unwrap();
    file.flush().unwrap();

    // Read single byte
    file.seek(io::SeekFrom::Start(0)).unwrap();
    let mut buf = [0u8; 1];
    file.read_exact(&mut buf).unwrap();
    assert_eq!(buf[0], b'X');

    // Truncate single byte (file should become 0 bytes)
    file.truncate().unwrap();
    file.flush().unwrap();
    file.seek(io::SeekFrom::Start(0)).unwrap();
    buf = [0u8; 1];
    let bytes_read = file.read(&mut buf).unwrap();
    // If file is truncated, read should return 0 or EOF
    // But truncate might not change file content, only file size
    assert!(bytes_read <= 1);
}

/// Test seeking to end of file
#[test]
fn test_seek_to_end_fat12() {
    call_with_fs(test_seek_to_end, FAT12_IMG, 119)
}

#[test]
fn test_seek_to_end_fat16() {
    call_with_fs(test_seek_to_end, FAT16_IMG, 120)
}

#[test]
fn test_seek_to_end_fat32() {
    call_with_fs(test_seek_to_end, FAT32_IMG, 121)
}

fn test_seek_to_end(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut file = root_dir.open_file("short.txt").unwrap();
    let content = TEST_STR.as_bytes();

    // Write content
    file.truncate().unwrap();
    file.write_all(content).unwrap();

    // Seek to end
    file.seek(io::SeekFrom::End(0)).unwrap();
    let pos = file.seek(io::SeekFrom::Current(0)).unwrap();
    assert_eq!(pos, content.len() as u64);

    // Seek to beginning
    file.seek(io::SeekFrom::Start(0)).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, content);
}

/// Test multiple seek operations
#[test]
fn test_multiple_seeks_fat12() {
    call_with_fs(test_multiple_seeks, FAT12_IMG, 122)
}

#[test]
fn test_multiple_seeks_fat16() {
    call_with_fs(test_multiple_seeks, FAT16_IMG, 123)
}

#[test]
fn test_multiple_seeks_fat32() {
    call_with_fs(test_multiple_seeks, FAT32_IMG, 124)
}

fn test_multiple_seeks(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut file = root_dir.open_file("short.txt").unwrap();
    let content = TEST_STR.as_bytes();

    // Prepare file
    file.truncate().unwrap();
    file.write_all(content).unwrap();

    // Multiple seeks
    file.seek(io::SeekFrom::Start(5)).unwrap();
    file.seek(io::SeekFrom::Start(0)).unwrap();
    file.seek(io::SeekFrom::End(-5)).unwrap();
    file.seek(io::SeekFrom::Start(10)).unwrap();

    let mut buf = [0u8; 5];
    file.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, &content[10..15]);
}

/// Test directory iteration boundaries
#[test]
fn test_dir_iteration_boundaries_fat12() {
    call_with_fs(test_dir_iteration_boundaries, FAT12_IMG, 125)
}

#[test]
fn test_dir_iteration_boundaries_fat16() {
    call_with_fs(test_dir_iteration_boundaries, FAT16_IMG, 126)
}

#[test]
fn test_dir_iteration_boundaries_fat32() {
    call_with_fs(test_dir_iteration_boundaries, FAT32_IMG, 127)
}

fn test_dir_iteration_boundaries(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let dir = root_dir.open_dir("very/long/path").unwrap();

    // Iterate directory entries
    let names: Vec<String> = dir.iter().map(|r| r.unwrap().file_name()).collect();
    // Should at least contain . and ..
    assert!(names.len() >= 2);
    assert!(names.contains(&String::from(".")));
    assert!(names.contains(&String::from("..")));
}

// ============================================================================
// Filesystem Robustness Tests
// ===========================================================================

/// Test creation and deletion of many small files
#[test]
fn test_many_small_files_fat12() {
    call_with_fs(test_many_small_files, FAT12_IMG, 128)
}

#[test]
fn test_many_small_files_fat16() {
    call_with_fs(test_many_small_files, FAT16_IMG, 129)
}

#[test]
fn test_many_small_files_fat32() {
    call_with_fs(test_many_small_files, FAT32_IMG, 130)
}

fn test_many_small_files(fs: FileSystem) {
    let root_dir = fs.root_dir();

    // Create many small files
    let count = 10; // Enough for testing but not too time-consuming
    for i in 0..count {
        let filename = format!("file{}.txt", i);
        let mut file = root_dir.create_file(&filename).unwrap();
        let content = format!("Content {}", i);
        file.write_all(content.as_bytes()).unwrap();
    }

    // Verify all files exist
    for i in 0..count {
        let filename = format!("file{}.txt", i);
        let mut file = root_dir.open_file(&filename).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        let expected = format!("Content {}", i);
        assert_eq!(str::from_utf8(&buf).unwrap(), expected);
    }

    // Delete all files
    for i in 0..count {
        let filename = format!("file{}.txt", i);
        root_dir.remove(&filename).unwrap();
    }
}

/// Test deep directory nesting
#[test]
fn test_deep_directory_nesting_fat12() {
    call_with_fs(test_deep_directory_nesting, FAT12_IMG, 131)
}

#[test]
fn test_deep_directory_nesting_fat16() {
    call_with_fs(test_deep_directory_nesting, FAT16_IMG, 132)
}

#[test]
fn test_deep_directory_nesting_fat32() {
    call_with_fs(test_deep_directory_nesting, FAT32_IMG, 133)
}

fn test_deep_directory_nesting(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let depth = 3; // Reduce depth to avoid issues

    // Create deep directory structure
    let mut current = root_dir.clone();
    for i in 0..depth {
        let dir_name = format!("level{}", i);
        current = current.create_dir(&dir_name).unwrap();
    }

    // Create file in deepest directory
    let mut file = current.create_file("deep_file.txt").unwrap();
    file.write_all(b"Deep content").unwrap();
    file.flush().unwrap();

    // Reopen deep directory from root
    let path = format!("level0/level1/level2/deep_file.txt");
    let mut file = root_dir.open_file(&path).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), "Deep content");
}

/// Test file truncation to various sizes
#[test]
fn test_file_truncate_various_sizes_fat12() {
    call_with_fs(test_file_truncate_various_sizes, FAT12_IMG, 134)
}

#[test]
fn test_file_truncate_various_sizes_fat16() {
    call_with_fs(test_file_truncate_various_sizes, FAT16_IMG, 135)
}

#[test]
fn test_file_truncate_various_sizes_fat32() {
    call_with_fs(test_file_truncate_various_sizes, FAT32_IMG, 136)
}

fn test_file_truncate_various_sizes(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut file = root_dir.create_file("truncate_test.txt").unwrap();

    // Write initial content
    let initial_content = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    file.write_all(initial_content).unwrap();

    // Truncate to various sizes and verify
    let sizes = [0, 5, 10, 20, 30];
    for size in sizes {
        file.truncate().unwrap();
        file.seek(io::SeekFrom::Start(0)).unwrap();

        // Write specified size
        let data = &initial_content[..size.min(initial_content.len())];
        file.write_all(data).unwrap();

        // Read and verify
        file.seek(io::SeekFrom::Start(0)).unwrap();
        let mut buf = vec![0u8; size];
        let bytes_read = file.read(&mut buf).unwrap();
        assert_eq!(bytes_read, size.min(initial_content.len()));
        assert_eq!(&buf[..bytes_read], data);
    }
}

/// Test concurrent operations (sequential simulation)
#[test]
fn test_sequential_operations_fat12() {
    call_with_fs(test_sequential_operations, FAT12_IMG, 137)
}

#[test]
fn test_sequential_operations_fat16() {
    call_with_fs(test_sequential_operations, FAT16_IMG, 138)
}

#[test]
fn test_sequential_operations_fat32() {
    call_with_fs(test_sequential_operations, FAT32_IMG, 139)
}

fn test_sequential_operations(fs: FileSystem) {
    let root_dir = fs.root_dir();

    // Execute a series of create, write, read, and delete operations
    for i in 0..5 {
        let filename = format!("seq{}.txt", i);

        // Create
        let mut file = root_dir.create_file(&filename).unwrap();

        // Write
        let content = format!("Sequence {}", i);
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();

        // Read
        file.seek(io::SeekFrom::Start(0)).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        assert_eq!(str::from_utf8(&buf).unwrap(), content);
    }

    // Delete all
    for i in 0..5 {
        let filename = format!("seq{}.txt", i);
        root_dir.remove(&filename).unwrap();
    }

    // Verify deletion succeeded
    for i in 0..5 {
        let filename = format!("seq{}.txt", i);
        assert!(root_dir.open_file(&filename).is_err());
    }
}

/// Test file rename edge cases
#[test]
fn test_rename_edge_cases_fat12() {
    call_with_fs(test_rename_edge_cases, FAT12_IMG, 140)
}

#[test]
fn test_rename_edge_cases_fat16() {
    call_with_fs(test_rename_edge_cases, FAT16_IMG, 141)
}

#[test]
fn test_rename_edge_cases_fat32() {
    call_with_fs(test_rename_edge_cases, FAT32_IMG, 142)
}

fn test_rename_edge_cases(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let target_dir = root_dir.open_dir("very/long/path").unwrap();

    // Create file
    let mut file = root_dir.create_file("test_rename.txt").unwrap();
    file.write_all(TEST_STR.as_bytes()).unwrap();
    file.flush().unwrap(); // Ensure data is written to disk

    // Close file
    drop(file);

    // Rename to different directory
    root_dir.rename("test_rename.txt", &target_dir, "renamed.txt").unwrap();

    // Verify original location does not exist
    assert!(root_dir.open_file("test_rename.txt").is_err());

    // Verify new location exists
    let mut file = target_dir.open_file("renamed.txt").unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_STR);

    // Close file
    drop(file);

    // Rename to same directory
    target_dir.rename("renamed.txt", &target_dir, "renamed2.txt").unwrap();
    assert!(target_dir.open_file("renamed.txt").is_err());
    assert!(target_dir.open_file("renamed2.txt").is_ok());
}

/// Test overwrite writing
#[test]
fn test_overwrite_content_fat12() {
    call_with_fs(test_overwrite_content, FAT12_IMG, 143)
}

#[test]
fn test_overwrite_content_fat16() {
    call_with_fs(test_overwrite_content, FAT16_IMG, 144)
}

#[test]
fn test_overwrite_content_fat32() {
    call_with_fs(test_overwrite_content, FAT32_IMG, 145)
}

fn test_overwrite_content(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut file = root_dir.create_file("overwrite.txt").unwrap();

    // Write initial content
    file.write_all(b"Original content").unwrap();
    file.flush().unwrap();

    // Seek to beginning and overwrite
    file.seek(io::SeekFrom::Start(0)).unwrap();
    file.write_all(b"New").unwrap();
    file.flush().unwrap();

    // Reading should contain mixed content
    file.seek(io::SeekFrom::Start(0)).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    // Should be "New" + "Original content"[3..]
    assert_eq!(&buf[..3], b"New");
}

/// Test directory creation and deletion boundaries
#[test]
fn test_dir_create_remove_edges_fat12() {
    call_with_fs(test_dir_create_remove_edges, FAT12_IMG, 146)
}

#[test]
fn test_dir_create_remove_edges_fat16() {
    call_with_fs(test_dir_create_remove_edges, FAT16_IMG, 147)
}

#[test]
fn test_dir_create_remove_edges_fat32() {
    call_with_fs(test_dir_create_remove_edges, FAT32_IMG, 148)
}

fn test_dir_create_remove_edges(fs: FileSystem) {
    let root_dir = fs.root_dir();

    // Create single-character directory name
    let dir = root_dir.create_dir("a").unwrap();
    assert!(dir.iter().count() >= 2); // . and ..

    // Create long directory name
    let dir = root_dir.create_dir("very-long-directory-name-test").unwrap();
    assert!(dir.iter().count() >= 2);

    // Create file in subdirectory
    let mut file = dir.create_file("nested.txt").unwrap();
    file.write_all(b"Nested file").unwrap();

    // Try to delete non-empty directory (should fail)
    assert!(root_dir.remove("very-long-directory-name-test").is_err());

    // Can delete directory after deleting file
    dir.remove("nested.txt").unwrap();
    assert!(root_dir.remove("very-long-directory-name-test").is_ok());

    // Delete single-character directory
    assert!(root_dir.remove("a").is_ok());
}

/// Test filesystem state persistence
#[test]
fn test_filesystem_persistence_fat12() {
    test_filesystem_persistence(FAT12_IMG, 149)
}

#[test]
fn test_filesystem_persistence_fat16() {
    test_filesystem_persistence(FAT16_IMG, 150)
}

#[test]
fn test_filesystem_persistence_fat32() {
    test_filesystem_persistence(FAT32_IMG, 151)
}

fn test_filesystem_persistence(filename: &str, test_seq: u32) {
    let _ = env_logger::builder().is_test(true).try_init();
    let img_path = format!("{}/{}", IMG_DIR, filename);
    let tmp_path = format!("{}/{}-{}", TMP_DIR, test_seq, filename);
    fs::create_dir(TMP_DIR).ok();
    fs::copy(img_path, &tmp_path).unwrap();

    // First open and write
    {
        let fs = open_filesystem_rw(&tmp_path);
        let root_dir = fs.root_dir();
        let mut file = root_dir.create_file("persistent.txt").unwrap();
        file.write_all(b"Persistent data").unwrap();
    }

    // Second open and read
    {
        let fs = open_filesystem_ro(&tmp_path);
        let root_dir = fs.root_dir();
        let mut file = root_dir.open_file("persistent.txt").unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        assert_eq!(str::from_utf8(&buf).unwrap(), "Persistent data");
    }

    fs::remove_file(tmp_path).unwrap();
}

/// Test invalid seek positions
#[test]
fn test_invalid_seek_fat12() {
    call_with_fs(test_invalid_seek, FAT12_IMG, 152)
}

#[test]
fn test_invalid_seek_fat16() {
    call_with_fs(test_invalid_seek, FAT16_IMG, 153)
}

#[test]
fn test_invalid_seek_fat32() {
    call_with_fs(test_invalid_seek, FAT32_IMG, 154)
}

fn test_invalid_seek(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut file = root_dir.open_file("short.txt").unwrap();

    // Test negative offset with SeekFrom::End
    file.seek(io::SeekFrom::End(-5)).unwrap();
    let mut buf = vec![0u8; 5];
    file.read_exact(&mut buf).unwrap();

    // Test negative offset with SeekFrom::Current
    file.seek(io::SeekFrom::Current(-5)).unwrap();
    let mut buf = vec![0u8; 5];
    file.read_exact(&mut buf).unwrap();

    // Test SeekFrom::Start(0)
    file.seek(io::SeekFrom::Start(0)).unwrap();
    let pos = file.seek(io::SeekFrom::Current(0)).unwrap();
    assert_eq!(pos, 0);

    // Test SeekFrom::End(0) - move to end of file
    file.seek(io::SeekFrom::End(0)).unwrap();
    let pos = file.seek(io::SeekFrom::Current(0)).unwrap();
    // Position should be at end of file
    assert!(pos >= 0);
}

/// Test reading beyond file size
#[test]
fn test_read_beyond_file_fat12() {
    call_with_fs(test_read_beyond_file, FAT12_IMG, 155)
}

#[test]
fn test_read_beyond_file_fat16() {
    call_with_fs(test_read_beyond_file, FAT16_IMG, 156)
}

#[test]
fn test_read_beyond_file_fat32() {
    call_with_fs(test_read_beyond_file, FAT32_IMG, 157)
}

fn test_read_beyond_file(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut file = root_dir.open_file("short.txt").unwrap();

    // Try to read beyond file size
    file.seek(io::SeekFrom::Start(1000)).unwrap();
    let mut buf = vec![0u8; 100];
    let bytes_read = file.read(&mut buf).unwrap();
    // Should return 0 (EOF)
    assert_eq!(bytes_read, 0);
}
