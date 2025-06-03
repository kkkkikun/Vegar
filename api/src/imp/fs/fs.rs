use alloc::sync::Arc;
use arceos_posix_api::{Directory, File, FileLike};
use axerrno::{LinuxError, LinuxResult};
use axfs::fops;
use axfs::fops::OpenOptions;

pub fn open_file(path: &str, options: Option<OpenOptions>) -> LinuxResult<File> {
    let options = options.unwrap_or_else(|| {
        let mut options = OpenOptions::new();
        options.read(true);
        options
    });
    let file_inner = fops::File::open(path, &options)?;
    Ok(File::new(file_inner, path))
}

pub fn open_file_like(path: &str, options: Option<OpenOptions>) -> LinuxResult<Arc<dyn FileLike>> {
    let options = options.unwrap_or_else(|| {
        let mut options = OpenOptions::new();
        options.read(true);
        options
    });
    if let Ok(file_inner) = fops::File::open(path, &options) {
        Ok(Arc::new(File::new(file_inner, path)))
    } else if let Ok(dir_inner) = fops::Directory::open_dir(path, &options) {
        Ok(Arc::new(Directory::new(dir_inner, path)))
    } else {
        return Err(LinuxError::ENOENT);
    }
}
