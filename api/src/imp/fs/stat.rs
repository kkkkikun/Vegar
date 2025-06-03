use axerrno::LinuxResult;
use core::ffi::c_char;
use macro_rules_attribute::apply;

use crate::imp::fs::status::TimeSpec;
use crate::{
    ptr::{PtrWrapper, UserConstPtr, UserPtr},
    syscall_instrument,
};

#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct Kstat {
    /// 设备
    pub st_dev: u64,
    /// inode 编号
    pub st_ino: u64,
    /// 文件类型
    pub st_mode: u32,
    /// 硬链接数
    pub st_nlink: u32,
    /// 用户id
    pub st_uid: u32,
    /// 用户组id
    pub st_gid: u32,
    /// 设备号
    pub st_rdev: u64,
    /// padding
    pub _pad0: u64,
    /// 文件大小
    pub st_size: u64,
    /// 块大小
    pub st_blksize: u32,
    /// padding
    pub _pad1: u32,
    /// 块个数
    pub st_blocks: u64,
    /// 最后一次访问时间(秒)
    pub st_atime_sec: isize,
    /// 最后一次访问时间(纳秒)
    pub st_atime_nsec: isize,
    /// 最后一次修改时间(秒)
    pub st_mtime_sec: isize,
    /// 最后一次修改时间(纳秒)
    pub st_mtime_nsec: isize,
    /// 最后一次改变状态时间(秒)
    pub st_ctime_sec: isize,
    /// 最后一次改变状态时间(纳秒)
    pub st_ctime_nsec: isize,
}

impl From<arceos_posix_api::ctypes::stat> for Kstat {
    fn from(stat: arceos_posix_api::ctypes::stat) -> Self {
        Self {
            st_dev: stat.st_dev,
            st_ino: stat.st_ino,
            st_mode: stat.st_mode,
            st_nlink: stat.st_nlink,
            st_uid: stat.st_uid,
            st_gid: stat.st_gid,
            st_rdev: stat.st_rdev,
            _pad0: 0,
            st_size: stat.st_size as u64,
            st_blksize: stat.st_blksize as u32,
            _pad1: 0,
            st_blocks: stat.st_blocks as u64,
            st_atime_sec: stat.st_atime.tv_sec as isize,
            st_atime_nsec: stat.st_atime.tv_nsec as isize,
            st_mtime_sec: stat.st_mtime.tv_sec as isize,
            st_mtime_nsec: stat.st_mtime.tv_nsec as isize,
            st_ctime_sec: stat.st_ctime.tv_sec as isize,
            st_ctime_nsec: stat.st_ctime.tv_nsec as isize,
        }
    }
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct FsStatxTimestamp {
    pub tv_sec: i64,
    pub tv_nsec: u32,
}

impl From<TimeSpec> for FsStatxTimestamp {
    fn from(ts: TimeSpec) -> Self {
        Self {
            tv_sec: ts.seconds as i64,
            tv_nsec: ts.nanoseconds as u32,
        }
    }
}

/// statfs - get filesystem statistics
/// Standard C library (libc, -lc)
/// <https://man7.org/linux/man-pages/man2/statfs.2.html>
#[repr(C)]
#[derive(Debug, Default)]
pub struct StatFs {
    /// Type of filesystem (see below)
    pub f_type: FsWord,
    /// Optimal transfer block size
    pub f_bsize: FsWord,
    /// Total data blocks in filesystem
    pub f_blocks: FsBlkCnt,
    /// Free blocks in filesystem
    pub f_bfree: FsBlkCnt,
    /// Free blocks available to unprivileged user
    pub f_bavail: FsBlkCnt,
    /// Total inodes in filesystem
    pub f_files: FsFilCnt,
    /// Free inodes in filesystem
    pub f_ffree: FsFilCnt,
    /// Filesystem ID
    pub f_fsid: FsId,
    /// Maximum length of filenames
    pub f_namelen: FsWord,
    /// Fragment size (since Linux 2.6)
    pub f_frsize: FsWord,
    /// Mount flags of filesystem (since Linux 2.6.36)
    pub f_flags: FsWord,
    /// Padding bytes reserved for future use
    pub f_spare: [FsWord; 5],
}

/// Type of miscellaneous file system fields. (typedef long __fsword_t)
pub type FsWord = isize;

/// Type to count file system blocks. (typedef unsigned long __fsblkcnt_t)
pub type FsBlkCnt = usize;

/// Type to count file system nodes. (typedef unsigned long __fsfilcnt_t)
pub type FsFilCnt = usize;

/// Type of file system IDs.
#[repr(C)]
#[derive(Debug, Default)]
pub struct FsId {
    /// raw value of the ID
    pub val: [i32; 2],
}

pub struct FsType;

impl FsType {
    const EXT4_SUPER_MAGIC: u32 = 0xEF53;
}

// TODO: [dummy] return dummy values
#[apply(syscall_instrument)]
pub fn sys_statfs(path: UserConstPtr<c_char>, buf: UserPtr<StatFs>) -> LinuxResult<isize> {
    let path = path.get_as_str()?;
    let _ = arceos_posix_api::handle_file_path(-1, Some(path.as_ptr() as _), false)?;

    // dummy data
    let stat_fs = StatFs {
        f_type: FsType::EXT4_SUPER_MAGIC as _,
        f_bsize: 4096,
        f_namelen: 255,
        f_frsize: 4096,
        f_blocks: 100000,
        f_bfree: 50000,
        f_bavail: 40000,
        f_files: 1000,
        f_ffree: 500,
        ..Default::default()
    };

    unsafe {
        buf.get()?.write(stat_fs);
    }
    Ok(0)
}
