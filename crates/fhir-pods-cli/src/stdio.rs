use std::io::{self, Write};

// Rust's standard streams can treat an invalid descriptor as an empty
// stream. Writing through File preserves EBADF (including a read-only
// output descriptor). The original descriptor stays open. Rust's startup
// code has already reopened inherited closed descriptors on /dev/null;
// those successful discarded writes cannot be distinguished here.
#[cfg(unix)]
pub fn write_line(text: &str, stderr: bool) -> io::Result<()> {
    use std::os::fd::FromRawFd;
    unsafe extern "C" {
        fn dup(fd: std::ffi::c_int) -> std::ffi::c_int;
    }
    // SAFETY: dup accepts an integer descriptor and returns an owned new
    // descriptor or -1. It accepts closed descriptors and reports EBADF.
    let fd = unsafe { dup(if stderr { 2 } else { 1 }) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful dup returned this live descriptor, owned only here.
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd) };
    writeln!(stream, "{text}")
}

#[cfg(not(unix))]
pub fn write_line(text: &str, stderr: bool) -> io::Result<()> {
    if stderr {
        writeln!(io::stderr().lock(), "{text}")
    } else {
        writeln!(io::stdout().lock(), "{text}")
    }
}
