/// Error enum with all errors that can be returned by functions from this crate.
///
/// This enum represents all possible errors that can occur when performing
/// operations on a FAT filesystem.

#[derive(Debug)]
#[non_exhaustive]
pub enum Error<T> {
    /// A user provided storage instance returned an error during an input/output operation.
    Io(T),
    /// A read operation cannot be completed because an end of a file has been reached prematurely.
    UnexpectedEof,
    /// A write operation cannot be completed because `Write::write` returned 0.
    WriteZero,
    /// A parameter was incorrect.
    InvalidInput,
    /// A requested file or directory has not been found.
    NotFound,
    /// A file or a directory with the same name already exists.
    AlreadyExists,
    /// An operation cannot be finished because a directory is not empty.
    DirectoryIsNotEmpty,
    /// File system internal structures are corrupted/invalid.
    CorruptedFileSystem,
    /// There is not enough free space on the storage to finish the requested operation.
    NotEnoughSpace,
    /// The provided file name is either too long or empty.
    InvalidFileNameLength,
    /// The provided file name contains an invalid character.
    UnsupportedFileNameCharacter,
}

impl<T: IoError> From<T> for Error<T> {
    fn from(error: T) -> Self {
        Error::Io(error)
    }
}

#[cfg(feature = "std")]
impl From<Error<std::io::Error>> for std::io::Error {
    fn from(error: Error<Self>) -> Self {
        match error {
            Error::Io(io_error) => io_error,
            Error::UnexpectedEof | Error::NotEnoughSpace => Self::new(std::io::ErrorKind::UnexpectedEof, error),
            Error::WriteZero => Self::new(std::io::ErrorKind::WriteZero, error),
            Error::InvalidInput
            | Error::InvalidFileNameLength
            | Error::UnsupportedFileNameCharacter
            | Error::DirectoryIsNotEmpty => Self::new(std::io::ErrorKind::InvalidInput, error),
            Error::NotFound => Self::new(std::io::ErrorKind::NotFound, error),
            Error::AlreadyExists => Self::new(std::io::ErrorKind::AlreadyExists, error),
            Error::CorruptedFileSystem => Self::new(std::io::ErrorKind::InvalidData, error),
        }
    }
}

impl<T: core::fmt::Display> core::fmt::Display for Error<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Io(io_error) => write!(f, "IO error: {}", io_error),
            Error::UnexpectedEof => write!(f, "Unexpected end of file"),
            Error::NotEnoughSpace => write!(f, "Not enough space"),
            Error::WriteZero => write!(f, "Write zero"),
            Error::InvalidInput => write!(f, "Invalid input"),
            Error::InvalidFileNameLength => write!(f, "Invalid file name length"),
            Error::UnsupportedFileNameCharacter => write!(f, "Unsupported file name character"),
            Error::DirectoryIsNotEmpty => write!(f, "Directory is not empty"),
            Error::NotFound => write!(f, "No such file or directory"),
            Error::AlreadyExists => write!(f, "File or directory already exists"),
            Error::CorruptedFileSystem => write!(f, "Corrupted file system"),
        }
    }
}

#[cfg(feature = "std")]
impl<T: std::error::Error + 'static> std::error::Error for Error<T> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Error::Io(io_error) = self {
            Some(io_error)
        } else {
            None
        }
    }
}

/// Trait that should be implemented by errors returned from the user supplied storage.
///
/// Implementations for `std::io::Error` and `()` are provided by this crate.
pub trait IoError: core::fmt::Debug {
    /// Checks if an operation was interrupted.
    ///
    /// Returns `true` if the error indicates that an operation was interrupted,
    /// typically allowing for the operation to be retried.
    ///
    /// # Returns
    ///
    /// `true` if this is an interruption error, `false` otherwise.
    fn is_interrupted(&self) -> bool;

    /// Creates a new error representing unexpected end of file.
    ///
    /// This is used internally by the library when a read operation fails to
    /// read the expected number of bytes.
    ///
    /// # Returns
    ///
    /// A new instance of the error type.
    fn new_unexpected_eof_error() -> Self;

    /// Creates a new error representing a write operation that wrote zero bytes.
    ///
    /// This is used internally by the library when a write operation fails to
    /// write any bytes to storage.
    ///
    /// # Returns
    ///
    /// A new instance of the error type.
    fn new_write_zero_error() -> Self;
}

impl<T: core::fmt::Debug + IoError> IoError for Error<T> {
    fn is_interrupted(&self) -> bool {
        match self {
            Error::<T>::Io(io_error) => io_error.is_interrupted(),
            _ => false,
        }
    }

    fn new_unexpected_eof_error() -> Self {
        Error::<T>::UnexpectedEof
    }

    fn new_write_zero_error() -> Self {
        Error::<T>::WriteZero
    }
}

impl IoError for () {
    fn is_interrupted(&self) -> bool {
        false
    }

    fn new_unexpected_eof_error() -> Self {
        // empty
    }

    fn new_write_zero_error() -> Self {
        // empty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_error_unit() {
        assert!(!().is_interrupted());
        let _: () = <() as IoError>::new_unexpected_eof_error();
        let _: () = <() as IoError>::new_write_zero_error();
    }

    #[test]
    fn test_io_error_wrapper() {
        let inner = std::io::Error::new(std::io::ErrorKind::Interrupted, "interrupted");
        let error = Error::Io(inner);
        assert!(error.is_interrupted());

        let inner = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let error = Error::Io(inner);
        assert!(!error.is_interrupted());
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_error_into_std_io_error() {
        let error = Error::<std::io::Error>::UnexpectedEof;
        let std_error: std::io::Error = error.into();
        assert_eq!(std_error.kind(), std::io::ErrorKind::UnexpectedEof);

        let error = Error::<std::io::Error>::WriteZero;
        let std_error: std::io::Error = error.into();
        assert_eq!(std_error.kind(), std::io::ErrorKind::WriteZero);

        let error = Error::<std::io::Error>::InvalidInput;
        let std_error: std::io::Error = error.into();
        assert_eq!(std_error.kind(), std::io::ErrorKind::InvalidInput);

        let error = Error::<std::io::Error>::NotFound;
        let std_error: std::io::Error = error.into();
        assert_eq!(std_error.kind(), std::io::ErrorKind::NotFound);

        let error = Error::<std::io::Error>::AlreadyExists;
        let std_error: std::io::Error = error.into();
        assert_eq!(std_error.kind(), std::io::ErrorKind::AlreadyExists);

        let error = Error::<std::io::Error>::CorruptedFileSystem;
        let std_error: std::io::Error = error.into();
        assert_eq!(std_error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_error_source() {
        use std::error::Error as StdError;

        let inner = std::io::Error::new(std::io::ErrorKind::NotFound, "test");
        let error = Error::Io(inner);
        assert!(StdError::source(&error).is_some());

        let error = Error::<std::io::Error>::NotFound;
        assert!(StdError::source(&error).is_none());
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_io_error_std() {
        use std::io::{Error, ErrorKind};

        let error = Error::new(ErrorKind::Interrupted, "interrupted");
        assert!(error.is_interrupted());

        let error = Error::new(ErrorKind::NotFound, "not found");
        assert!(!error.is_interrupted());

        let error = Error::new_unexpected_eof_error();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);

        let error = Error::new_write_zero_error();
        assert_eq!(error.kind(), ErrorKind::WriteZero);
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_error_display_with_std_error() {
        assert_eq!(
            format!("{}", Error::<std::io::Error>::UnexpectedEof),
            "Unexpected end of file"
        );
        assert_eq!(
            format!("{}", Error::<std::io::Error>::NotEnoughSpace),
            "Not enough space"
        );
        assert_eq!(format!("{}", Error::<std::io::Error>::WriteZero), "Write zero");
        assert_eq!(format!("{}", Error::<std::io::Error>::InvalidInput), "Invalid input");
        assert_eq!(
            format!("{}", Error::<std::io::Error>::InvalidFileNameLength),
            "Invalid file name length"
        );
        assert_eq!(
            format!("{}", Error::<std::io::Error>::UnsupportedFileNameCharacter),
            "Unsupported file name character"
        );
        assert_eq!(
            format!("{}", Error::<std::io::Error>::DirectoryIsNotEmpty),
            "Directory is not empty"
        );
        assert_eq!(
            format!("{}", Error::<std::io::Error>::NotFound),
            "No such file or directory"
        );
        assert_eq!(
            format!("{}", Error::<std::io::Error>::AlreadyExists),
            "File or directory already exists"
        );
        assert_eq!(
            format!("{}", Error::<std::io::Error>::CorruptedFileSystem),
            "Corrupted file system"
        );
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_error_io_display_with_std_error() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "test");
        assert_eq!(format!("{}", Error::<std::io::Error>::Io(io_error)), "IO error: test");
    }
}

#[cfg(feature = "std")]
impl IoError for std::io::Error {
    fn is_interrupted(&self) -> bool {
        self.kind() == std::io::ErrorKind::Interrupted
    }

    fn new_unexpected_eof_error() -> Self {
        Self::new(std::io::ErrorKind::UnexpectedEof, "failed to fill whole buffer")
    }

    fn new_write_zero_error() -> Self {
        Self::new(std::io::ErrorKind::WriteZero, "failed to write whole buffer")
    }
}
