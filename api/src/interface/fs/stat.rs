use crate::imp::fs::FsStatxTimestamp;
use crate::imp::fs::status::{FileStatus, TimeSpec, sys_stat_impl};
use crate::ptr::{PtrWrapper, UserInPtr, UserOutPtr};
use alloc::format;
use arceos_posix_api::AT_FDCWD;
use axerrno::LinuxError;
use axerrno::LinuxResult;
use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong};
use syscall_trace::syscall_trace;

// user constants
const AT_EMPTY_PATH: c_int = 0x1000;
const AT_SYMLINK_NOFOLLOW: c_int = 0x100;

/// user struct: stat
#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct UserStat {
    /// ID of device containing file
    pub st_dev: c_ulong,
    /// inode number
    pub st_ino: c_ulong,
    /// number of hard links
    /// note that the field sequence is different from other archs
    pub st_nlink: c_ulong,
    /// file type and mode
    pub st_mode: c_uint,
    /// user ID of owner
    pub st_uid: c_uint,
    /// group ID of owner
    pub st_gid: c_uint,
    /// paddings for arch x86_64
    pub _pad0: c_int,
    /// device ID (if special file)
    pub st_rdev: c_ulong,
    /// total size, in bytes
    pub st_size: c_long,
    /// Block size for filesystem I/O
    pub st_blksize: c_long,
    /// number of blocks allocated
    pub st_blocks: c_long,
    /// time of last access
    pub st_atime: TimeSpec,
    /// time of last modification
    pub st_mtime: TimeSpec,
    /// time of last status change
    pub st_ctime: TimeSpec,
    /// glibc reserved for arch x86_64
    pub _glibc_reserved: [c_long; 3],
}

#[cfg(target_arch = "x86_64")]
impl From<FileStatus> for UserStat {
    fn from(fs: FileStatus) -> Self {
        UserStat {
            st_dev: fs.dev as c_ulong,
            st_ino: fs.inode as c_ulong,
            st_nlink: fs.n_link as c_ulong,
            st_mode: fs.mode,
            st_uid: fs.uid,
            st_gid: fs.gid,
            st_rdev: fs.rdev as c_ulong,
            st_size: fs.size as c_long,
            st_blksize: fs.block_size as c_long,
            st_blocks: fs.n_blocks as c_long,
            st_atime: fs.access_time,
            st_mtime: fs.modify_time,
            st_ctime: fs.change_time,
            ..Default::default()
        }
    }
}

#[cfg(target_arch = "x86_64")]
const _: () = assert!(size_of::<UserStat>() == 144, "size of Stat is not 144");

#[cfg(not(target_arch = "x86_64"))]
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct UserStat {
    /// ID of device containing file
    pub st_dev: c_ulong,
    /// inode number
    pub st_ino: c_ulong,
    /// file type and mode
    pub st_mode: c_uint,
    /// number of hard links
    pub st_nlink: c_uint,
    /// user ID of owner
    pub st_uid: c_uint,
    /// group ID of owner
    pub st_gid: c_uint,
    /// device ID (if special file)
    pub st_rdev: c_ulong,
    /// paddings for arch non x86_64
    pub _pad0: c_long,
    /// total size, in bytes
    pub st_size: c_long,
    /// Block size for filesystem I/O
    pub st_blksize: c_int,
    /// paddings for arch non x86_64
    pub _pad1: c_int,
    /// number of blocks allocated
    pub st_blocks: c_long,
    /// time of last access
    pub st_atime: TimeSpec,
    /// time of last modification
    pub st_mtime: TimeSpec,
    /// time of last status change
    pub st_ctime: TimeSpec,
    /// reserved for arch non x86_64
    pub _unused: [c_int; 2],
}

#[cfg(not(target_arch = "x86_64"))]
impl From<FileStatus> for UserStat {
    fn from(fs: FileStatus) -> Self {
        UserStat {
            st_dev: fs.dev as c_ulong,
            st_ino: fs.inode as c_ulong,
            st_mode: fs.mode,
            st_nlink: fs.n_link as c_uint,
            st_uid: fs.uid,
            st_gid: fs.gid,
            st_rdev: fs.rdev as c_ulong,
            st_size: fs.size as c_long,
            st_blksize: fs.block_size as c_int,
            st_blocks: fs.n_blocks as c_long,
            st_atime: fs.access_time,
            st_mtime: fs.modify_time,
            st_ctime: fs.change_time,
            ..Default::default()
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
const _: () = assert!(size_of::<UserStat>() == 128, "size of Stat is not 128");

/// user struct: statx
#[repr(C)]
#[derive(Debug, Default)]
pub struct UserStatX {
    /// Bitmask of what information to get.
    pub stx_mask: u32,
    /// Block size for filesystem I/O.
    pub stx_blksize: u32,
    /// File attributes.
    pub stx_attributes: u64,
    /// Number of hard links.
    pub stx_nlink: u32,
    /// User ID of owner.
    pub stx_uid: u32,
    /// Group ID of owner.
    pub stx_gid: u32,
    /// File mode (permissions).
    pub stx_mode: u16,
    /// padding
    pub _pad0: u16,
    /// Inode number.
    pub stx_ino: u64,
    /// Total size, in bytes.
    pub stx_size: u64,
    /// Number of 512B blocks allocated.
    pub stx_blocks: u64,
    /// Mask to show what's supported in stx_attributes.
    pub stx_attributes_mask: u64,
    /// Last access timestamp.
    pub stx_atime: FsStatxTimestamp,
    /// Birth (creation) timestamp.
    pub stx_btime: FsStatxTimestamp,
    /// Last status change timestamp.
    pub stx_ctime: FsStatxTimestamp,
    /// Last modification timestamp.
    pub stx_mtime: FsStatxTimestamp,
    /// Major device ID (if special file).
    pub stx_rdev_major: u32,
    /// Minor device ID (if special file).
    pub stx_rdev_minor: u32,
    /// Major device ID of file system.
    pub stx_dev_major: u32,
    /// Minor device ID of file system.
    pub stx_dev_minor: u32,
    /// Mount ID.
    pub stx_mnt_id: u64,
    /// Memory alignment for direct I/O.
    pub stx_dio_mem_align: u32,
    /// Offset alignment for direct I/O.
    pub stx_dio_offset_align: u32,
    /// Reserved for future use.
    pub _spare: [u32; 12],
}

impl From<FileStatus> for UserStatX {
    fn from(fs: FileStatus) -> Self {
        Self {
            stx_blksize: fs.block_size as _,
            stx_attributes: fs.mode as _,
            stx_nlink: fs.n_link as _,
            stx_uid: fs.uid,
            stx_gid: fs.gid,
            stx_mode: fs.mode as _,
            stx_ino: fs.inode as _,
            stx_size: fs.size as _,
            stx_blocks: fs.n_blocks as _,
            stx_attributes_mask: 0x7FF,
            stx_atime: fs.access_time.into(),
            stx_ctime: fs.change_time.into(),
            stx_mtime: fs.modify_time.into(),
            ..Default::default()
        }
    }
}

#[syscall_trace]
pub fn sys_stat(path: UserInPtr<c_char>, stat_buf: UserOutPtr<UserStat>) -> LinuxResult<isize> {
    // get params
    let path = path.get_as_str()?;
    let stat_buf = stat_buf.get()?;

    // perform syscall
    let file_status = sys_stat_impl(-1, path, false)?;
    unsafe { stat_buf.write(file_status.into()) }
    Ok(0)
}

/// TODO: ignored following symlinks
#[syscall_trace]
pub fn sys_lstat(path: UserInPtr<c_char>, stat_buf: UserOutPtr<UserStat>) -> LinuxResult<isize> {
    // get params
    let path = path.get_as_str()?;
    let stat_buf = stat_buf.get()?;

    // perform syscall
    let file_status = sys_stat_impl(-1, path, true)?;
    unsafe { stat_buf.write(file_status.into()) }
    Ok(0)
}

#[syscall_trace]
pub fn sys_fstat(fd: c_int, stat_buf: UserOutPtr<UserStat>) -> LinuxResult<isize> {
    // get params
    let stat_buf = stat_buf.get()?;

    // perform syscall
    if fd < 0 && fd != AT_FDCWD as i32 {
        return Err(LinuxError::EBADFD);
    }
    let file_status = sys_stat_impl(fd, "", false)?;
    unsafe { stat_buf.write(file_status.into()) }
    Ok(0)
}

#[syscall_trace]
pub fn sys_fstatat(
    dir_fd: c_int,
    path: UserInPtr<c_char>,
    stat_buf: UserOutPtr<UserStat>,
    flags: c_int,
) -> LinuxResult<isize> {
    // get params
    let path = path.get_as_str().unwrap_or("");
    let stat_buf = stat_buf.get()?;

    // perform syscall
    if dir_fd < 0 && dir_fd != AT_FDCWD as i32 {
        return Err(LinuxError::EBADFD);
    }
    // TODO: some flags are ignored
    if path.is_empty() && (flags & AT_EMPTY_PATH == 0) {
        return Err(LinuxError::ENOENT);
    }
    let follow_symlinks = flags & AT_SYMLINK_NOFOLLOW == 0;
    let file_status = sys_stat_impl(dir_fd, path, follow_symlinks)?;

    // write result
    unsafe { stat_buf.write(file_status.into()) }
    Ok(0)
}

#[syscall_trace]
pub fn sys_statx(
    dir_fd: c_int,
    path: UserInPtr<c_char>,
    flags: c_int,
    _mask: c_uint,
    statx_buf: UserOutPtr<UserStatX>,
) -> LinuxResult<isize> {
    // get params
    let path = path.get_as_str().unwrap_or("");
    let statx_buf = statx_buf.get()?;

    // perform syscall
    if dir_fd < 0 && dir_fd != AT_FDCWD as i32 {
        return Err(LinuxError::EBADFD);
    }
    // TODO: some flags are ignored
    if path.is_empty() && (flags & AT_EMPTY_PATH == 0) {
        return Err(LinuxError::ENOENT);
    }
    let follow_symlinks = flags & AT_SYMLINK_NOFOLLOW == 0;
    let file_status = sys_stat_impl(dir_fd, path, follow_symlinks)?;
    // TODO: add more fields
    unsafe { statx_buf.write(file_status.into()) }
    Ok(0)
}
