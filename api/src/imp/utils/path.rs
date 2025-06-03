use alloc::string::ToString;
use arceos_posix_api::{Directory, File, FilePath, get_file_like};
use axerrno::{LinuxError, LinuxResult};
use axfs::api::current_dir;

/// get `FilePath` from parent directory fd and path string
/// - if `path` is empty, return the file or directory specified by the `parent_fd`
/// - if `path` is absolute, `parent_fd` will be ignored
/// - if `path` is relative, `parent_fd` will be used to get the parent directory
/// - if `parent_fd` is negative, the parent directory will be the current working directory
pub fn resolve_path_with_parent(parent_fd: i32, path: &str) -> LinuxResult<FilePath> {
    // if `path` is empty, return the file or directory specified by the `parent_fd`
    if path.is_empty() {
        return if parent_fd < 0 {
            Ok(FilePath::new(current_dir()?)?)
        } else {
            resolve_path_from_fd(parent_fd)
        };
    }
    // if `path` is absolute, `parent_fd` will be ignored
    if path.starts_with('/') {
        return Ok(FilePath::new(path)?);
    }
    // if `path` is relative, `parent_fd` will be used to get the parent directory
    if parent_fd < 0 {
        // current working directory should end with '/'
        Ok(FilePath::new(current_dir()? + path)?)
    } else {
        if let Err(_) = get_file_like(parent_fd) {
            return Err(LinuxError::EBADF);
        }
        if let Ok(dir) = Directory::from_fd(parent_fd) {
            Ok(FilePath::new(dir.path().to_string() + path)?)
        } else {
            Err(LinuxError::ENOTDIR)
        }
    }
}

/// get `FilePath` from path string
/// if the path is relative, the current working directory will be used
pub fn resolve_path(path: &str) -> LinuxResult<FilePath> {
    resolve_path_with_parent(-1, path)
}

/// get `FilePath` of the file descriptor
pub fn resolve_path_from_fd(fd: i32) -> LinuxResult<FilePath> {
    let f = get_file_like(fd)?.into_any();
    let path: &str;
    if let Some(file) = f.downcast_ref::<File>() {
        path = file.path();
    } else if let Some(dir) = f.downcast_ref::<Directory>() {
        path = dir.path();
    } else {
        return Err(LinuxError::EBADF);
    }
    Ok(FilePath::new(path)?)
}
