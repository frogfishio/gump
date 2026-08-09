//! Bounded reads from explicitly inherited descriptors.
//!
//! The unsafe OS ownership conversion is isolated here. Callers receive a
//! safe read of a duplicated descriptor, so the caller's original remains
//! owned by its process boundary.

use std::io;

/// Duplicate `fd` and read at most `max_bytes` from the duplicate.
#[cfg(unix)]
pub fn read_bounded(fd: i32, max_bytes: usize) -> io::Result<Vec<u8>> {
    sys::read_bounded(fd, max_bytes)
}

#[cfg(not(unix))]
pub fn read_bounded(_fd: i32, _max_bytes: usize) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "inherited descriptors are unsupported on this platform",
    ))
}

#[cfg(unix)]
mod sys {
    #![allow(unsafe_code)]

    use std::fs::File;
    use std::io::{self, Read};
    use std::os::fd::FromRawFd;

    pub fn read_bounded(fd: i32, max_bytes: usize) -> io::Result<Vec<u8>> {
        // SAFETY: dup validates the supplied descriptor and returns a new,
        // independently owned descriptor or -1. Only the successful result is
        // converted into File ownership.
        let duplicate = unsafe { libc::dup(fd) };
        if duplicate < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `duplicate` was just returned by dup and is uniquely owned here.
        let file = unsafe { File::from_raw_fd(duplicate) };
        let mut bytes = Vec::with_capacity(max_bytes.min(4096));
        file.take(max_bytes as u64).read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Write as _;
    use std::os::fd::AsRawFd as _;
    use std::os::unix::net::UnixStream;

    #[test]
    fn reads_socket_descriptor_without_procfs_reopen() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"secret").unwrap();
        drop(writer);
        assert_eq!(
            super::read_bounded(reader.as_raw_fd(), 16).unwrap(),
            b"secret"
        );
    }
}
