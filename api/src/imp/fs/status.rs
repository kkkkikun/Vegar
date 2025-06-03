use crate::imp::fs::fs::open_file_like;
use crate::imp::utils::path::resolve_path_with_parent;
use arceos_posix_api::{ctypes, get_file_like};
use axerrno::LinuxResult;

/// File status
#[derive(Debug, Clone, Copy, Default)]
pub struct FileStatus {
    /// ID of device containing file
    pub dev: usize,
    /// inode number
    pub inode: usize,
    /// file type and mode
    pub mode: u32,
    /// number of hard links
    pub n_link: usize,
    /// user ID of owner
    pub uid: u32,
    /// group ID of owner
    pub gid: u32,
    /// device ID (if special file)
    pub rdev: usize,
    /// total size, in bytes
    pub size: isize,
    /// Block size for filesystem I/O
    pub block_size: isize,
    /// number of blocks allocated
    pub n_blocks: isize,
    /// time of last access
    pub access_time: TimeSpec,
    /// time of last modification
    pub modify_time: TimeSpec,
    /// time of last status change
    pub change_time: TimeSpec,
}

/// for compatibility with arceos_posix_api
impl From<ctypes::stat> for FileStatus {
    fn from(stat: ctypes::stat) -> Self {
        FileStatus {
            dev: stat.st_dev as _,
            inode: stat.st_ino as _,
            mode: stat.st_mode,
            n_link: stat.st_nlink as _,
            uid: stat.st_uid,
            gid: stat.st_gid,
            rdev: stat.st_rdev as _,
            size: stat.st_size as _,
            block_size: stat.st_blksize as _,
            n_blocks: stat.st_blocks as _,
            access_time: stat.st_atime.into(),
            modify_time: stat.st_mtime.into(),
            change_time: stat.st_ctime.into(),
            ..Default::default()
        }
    }
}

/// Time in seconds and nanoseconds
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct TimeSpec {
    /// seconds
    pub seconds: isize,
    /// nanoseconds in range [0, 999_999_999]
    pub nanoseconds: isize,
}

/// for compatibility with arceos_posix_api
impl From<ctypes::timespec> for TimeSpec {
    fn from(ts: ctypes::timespec) -> Self {
        TimeSpec {
            seconds: ts.tv_sec as _,
            nanoseconds: ts.tv_nsec as _,
        }
    }
}

/// syscall impl: get file status
/// [Availability] Most
/// TODO: add support for symlink
pub fn sys_stat_impl(dir_fd: i32, path: &str, _follow_symlinks: bool) -> LinuxResult<FileStatus> {
    let file = if path.is_empty() {
        get_file_like(dir_fd)?
    } else {
        let path = resolve_path_with_parent(dir_fd, path)?;
        open_file_like(path.as_str(), None)?
    };
    let file_status: FileStatus = file.stat()?.into();
    Ok(file_status)
}
