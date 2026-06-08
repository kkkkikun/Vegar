//! Virtual /lib filesystem for OSComp - provides dynamic linkers
//!
//! Serves architecture-specific musl and glibc dynamic linkers via a read-only
//! virtual filesystem mounted at `/lib`.

use alloc::{boxed::Box, sync::Arc};
use axfs_ng_vfs::{Filesystem, NodePermission, VfsError, VfsResult};
use axfs::FS_CONTEXT;

use super::{file::SimpleFile, fs::SimpleFs, DirMaker, NodeOpsMux};

// musl libc dynamic linker binary (stripped).
// Extracted from the musl libc official toolchain (libc.so).
// musl is MIT licensed: https://musl.libc.org/

#[cfg(target_arch = "riscv64")]
const LD_MUSL_ARCH: &[u8] = include_bytes!("ld_musl_riscv64.so.1");

#[cfg(target_arch = "loongarch64")]
const LD_MUSL_ARCH: &[u8] = include_bytes!("ld_musl_loongarch64.so.1");

const LD_MUSL: &[u8] = LD_MUSL_ARCH;

// glibc dynamic linker (ld-linux).
// Extracted from the OSComp test image at /glibc/lib/ld-linux-riscv64-lp64d.so.1.
// glibc is LGPL-2.1+: https://www.gnu.org/software/libc/

#[cfg(target_arch = "riscv64")]
const LD_LINUX: &[u8] = include_bytes!("ld_linux_riscv64_lp64d.so.1");

#[cfg(target_arch = "loongarch64")]
// TODO: extract and embed ld-linux-loongarch64-lp64d.so.1 for loongarch64
const LD_LINUX: &[u8] = &[];

const LD_LINUX_FILENAME: &str = if cfg!(target_arch = "riscv64") {
    "ld-linux-riscv64-lp64d.so.1"
} else if cfg!(target_arch = "loongarch64") {
    "ld-linux-loongarch64-lp64d.so.1"
} else {
    ""
};

#[cfg(target_arch = "riscv64")]
const LD_MUSL_FILENAME: &str = "ld-musl-riscv64.so.1";

#[cfg(target_arch = "loongarch64")]
const LD_MUSL_FILENAME: &str = "ld-musl-loongarch64.so.1";

/// Builder function for the libfs
fn builder(fs: Arc<SimpleFs>) -> DirMaker {
    Arc::new(move |weak_parent| {
        super::SimpleDir::new_maker(
            fs.clone(),
            Arc::new(LibDir { fs: fs.clone() }),
        )(weak_parent)
    })
}

/// Directory operations for /lib — serves both musl and glibc dynamic linkers
struct LibDir {
    fs: Arc<SimpleFs>,
}

impl super::SimpleDirOps for LibDir {
    fn child_names(&self) -> Box<dyn Iterator<Item = alloc::borrow::Cow<str>> + '_> {
        Box::new(
            [
                alloc::borrow::Cow::Borrowed(LD_MUSL_FILENAME),
                alloc::borrow::Cow::Borrowed(LD_LINUX_FILENAME),
            ]
            .into_iter(),
        )
    }

    fn lookup_child(&self, name: &str) -> VfsResult<NodeOpsMux> {
        let fs = self.fs.clone();
        Ok(match name {
            LD_MUSL_FILENAME => {
                SimpleFile::new_regular(fs.clone(), move || Ok(LD_MUSL.to_vec())).into()
            }
            LD_LINUX_FILENAME if !LD_LINUX.is_empty() => {
                SimpleFile::new_regular(fs.clone(), move || Ok(LD_LINUX.to_vec())).into()
            }
            _ => return Err(VfsError::NotFound),
        })
    }
}

/// Creates a new libfs that provides dynamic linkers at `/lib/`.
pub fn new_libfs() -> Filesystem {
    SimpleFs::new_with("lib".into(), 0x01021994, builder)
}

/// Mount the libfs at /lib with the dynamic linkers.
pub fn mount_libfs() -> VfsResult<()> {
    info!("Mounting libfs at /lib for dynamic linkers...");

    let fs = FS_CONTEXT.lock();

    if fs.resolve("/lib").is_err() {
        fs.create_dir("/lib", NodePermission::from_bits_truncate(0o755))?;
    }

    fs.resolve("/lib")?.mount(&new_libfs())?;

    info!(
        "Mounted dynamic linkers at /lib: {} ({} bytes), {} ({} bytes)",
        LD_MUSL_FILENAME,
        LD_MUSL.len(),
        LD_LINUX_FILENAME,
        LD_LINUX.len()
    );
    Ok(())
}
