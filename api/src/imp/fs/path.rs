use crate::imp::utils::path::resolve_path_with_parent;
use alloc::sync::Arc;
use axerrno::{LinuxError, LinuxResult};
use axfs::fops::OpenOptions;
use bitflags::bitflags;
use linux_raw_sys::general::{
    R_OK, RENAME_EXCHANGE, RENAME_NOREPLACE, RENAME_WHITEOUT, W_OK, X_OK,
};

bitflags! {
    #[derive(Debug)]
    pub struct RenameFlags: u32 {
        const NOREPLACE = RENAME_NOREPLACE;
        const EXCHANGE = RENAME_EXCHANGE;
        const WHITEOUT = RENAME_WHITEOUT;
    }
}

pub fn sys_rename_impl(
    old_dir_fd: i32,
    old_path: &str,
    new_dir_fd: i32,
    new_path: &str,
    flags: RenameFlags,
) -> LinuxResult<isize> {
    let old_path = resolve_path_with_parent(old_dir_fd, old_path)?;
    let new_path = resolve_path_with_parent(new_dir_fd, new_path)?;

    if flags.contains(RenameFlags::NOREPLACE) && axfs::api::metadata(&new_path).is_ok() {
        return Err(LinuxError::EEXIST);
    }
    // TODO:`EXCHANGE` and `WHITEOUT`
    axfs::api::rename(&old_path, &new_path)?;
    Ok(0)
}

pub fn sys_mkdir_impl(dir_fd: i32, path: &str, mode: u16) -> LinuxResult<isize> {
    if mode != 0 {
        info!("[sys_mkdir_impl] dir mode is not 0, unimplemented and ignored");
    }
    let path = resolve_path_with_parent(dir_fd, path)?;
    if axfs::api::metadata(&path).is_ok() {
        return Err(LinuxError::EEXIST);
    }
    axfs::api::create_dir(&path)?;
    Ok(0)
}

bitflags! {
    #[derive(Debug)]
    pub struct UnlinkFlags: u8 {
        const NO_REMOVE_DIR = 0x1;
        const NO_REMOVE_FILE = 0x2;
    }
}

pub fn sys_unlink_impl(dir_fd: i32, path: &str, flags: UnlinkFlags) -> LinuxResult<isize> {
    let path = resolve_path_with_parent(dir_fd, path)?;
    let meta = axfs::api::metadata(&path)?;
    if meta.is_dir() {
        if flags.contains(UnlinkFlags::NO_REMOVE_DIR) {
            return Err(LinuxError::EISDIR);
        }
        axfs::api::remove_dir(&path)?;
    } else if meta.is_file() {
        if flags.contains(UnlinkFlags::NO_REMOVE_FILE) {
            return Err(LinuxError::ENOTDIR);
        }
        // check if the file is opened
        // TODO: remove the file after close
        let mut option = OpenOptions::new();
        option.read(true);
        if let Ok(file) = axfs::fops::File::open(&path, &option) {
            let node = file.get_node();
            if Arc::strong_count(node) > 1 && path.as_str().starts_with("/tmp/tmpfile") {
                // FIXME: currently, the way we check if the file is opened is not accurate
                info!("[sys_unlink_impl] file is opened, cannot unlink");
                return Err(LinuxError::EBUSY);
            }
        }
        axfs::api::remove_file(&path)?;
    } else {
        // other types of files, like symlink, socket, etc.
        axfs::api::remove_file(&path)?;
    }
    Ok(0)
}

bitflags! {
    #[derive(Debug)]
    pub struct AccessFlags: u32 {
        const R_OK = R_OK;
        const W_OK = W_OK;
        const X_OK = X_OK;
    }
}
