use std::{ffi::CString, io, mem::MaybeUninit, os::unix::prelude::OsStrExt, path::PathBuf};

use libc::c_void;
use mockall::automock;
use tracing::error;

#[automock]
pub trait LibcWrapper {
    fn statfs(&self, path: PathBuf) -> io::Result<libc::statfs>;
    fn fstat(&self, fh: u64) -> io::Result<libc::stat>;
    fn lstat(&self, path: PathBuf) -> io::Result<libc::stat>;
    fn open(&self, path: PathBuf, flags: i32) -> io::Result<i32>;
    fn close(&self, fd: i32) -> io::Result<()>;
    fn read(&self, fd: i32, offset: i64, count: u32) -> io::Result<Vec<u8>>;
    fn unlink(&self, path: PathBuf) -> io::Result<()>;
}

pub struct LibcWrapperReal {}
impl LibcWrapperReal {
    pub fn new() -> Self {
        Self {}
    }
}
impl LibcWrapper for LibcWrapperReal {
    fn statfs(&self, path: PathBuf) -> io::Result<libc::statfs> {
        let mut stat = MaybeUninit::<libc::statfs>::zeroed();

        let cstr = CString::new(path.clone().into_os_string().as_bytes())?;
        let result = unsafe { libc::statfs(cstr.as_ptr(), stat.as_mut_ptr()) };

        if -1 == result {
            let e = io::Error::last_os_error();
            error!("statfs({:?}): {}", &path, e);
            Err(e)
        } else {
            let stat = unsafe { stat.assume_init() };
            Ok(stat)
        }
    }

    fn fstat(&self, fh: u64) -> io::Result<libc::stat> {
        let mut stat = MaybeUninit::<libc::stat>::uninit();

        let result = unsafe { libc::fstat(fh as libc::c_int, stat.as_mut_ptr()) };
        if -1 == result {
            let e = io::Error::last_os_error();
            error!("fstat({:?}): {}", fh, e);
            Err(e)
        } else {
            let stat = unsafe { stat.assume_init() };
            Ok(stat)
        }
    }

    fn lstat(&self, path: PathBuf) -> io::Result<libc::stat> {
        let mut stat = MaybeUninit::<libc::stat>::uninit();

        let cstr = CString::new(path.clone().into_os_string().as_bytes())?;
        let result = unsafe { libc::lstat(cstr.as_ptr(), stat.as_mut_ptr()) };
        if -1 == result {
            let e = io::Error::last_os_error();
            error!("lstat({:?}): {}", path, e);
            Err(e)
        } else {
            let stat = unsafe { stat.assume_init() };
            Ok(stat)
        }
    }

    fn open(&self, path: PathBuf, flags: i32) -> io::Result<i32> {
        let cstr = CString::new(path.clone().into_os_string().as_bytes())?;
        let result = unsafe { libc::open(cstr.as_ptr(), flags) };
        if -1 == result {
            let e = io::Error::last_os_error();
            error!("open({:?}): {}", path, e);
            Err(e)
        } else {
            Ok(result)
        }
    }

    fn close(&self, fd: i32) -> io::Result<()> {
        let result = unsafe { libc::close(fd) };
        if -1 == result {
            let e = io::Error::last_os_error();
            error!("close({:?}): {}", fd, e);
            Err(e)
        } else {
            Ok(())
        }
    }

    fn read(&self, fd: i32, offset: i64, count: u32) -> io::Result<Vec<u8>> {
        let result = unsafe { libc::lseek64(fd, offset, libc::SEEK_SET) };
        if -1 == result {
            let e = io::Error::last_os_error();
            error!("read({:?}): {}", fd, e);
            return Err(e);
        }
        let mut buf = Vec::new();
        buf.resize(count.try_into().unwrap(), 0);

        let result = unsafe {
            libc::read(
                fd,
                buf.as_mut_ptr() as *mut c_void,
                count.try_into().unwrap(),
            )
        };
        if -1 == result {
            let e = io::Error::last_os_error();
            error!("read({:?}): {}", fd, e);
            return Err(e);
        }
        Ok(buf)
    }

    fn unlink(&self, path: PathBuf) -> io::Result<()> {
        let cstr = CString::new(path.clone().into_os_string().as_bytes())?;
        let result = unsafe { libc::unlink(cstr.as_ptr()) };
        if -1 == result {
            let e = io::Error::last_os_error();
            error!("open({:?}): {}", path, e);
            Err(e)
        } else {
            Ok(())
        }
    }
}
