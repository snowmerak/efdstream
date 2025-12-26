use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd, BorrowedFd};
use std::process::{Child, Command, Stdio};
use std::os::unix::process::CommandExt;
use std::ptr::{self, NonNull};
use std::slice;

use nix::sys::eventfd::{EventFd, EfdFlags};
use nix::sys::mman::{mmap, munmap, MapFlags, ProtFlags};
use nix::sys::memfd::{memfd_create, MFdFlags};
use nix::unistd::ftruncate;
use std::ffi::CString;

pub struct ShmParent {
    child_path: String,
    fd_send: RawFd,
    fd_ack: RawFd,
    fd_shm: RawFd,
    file_send: Option<File>,
    file_ack: Option<File>,
    shm_file: Option<File>,
    shm_ptr: *mut u8,
    shm_size: usize,
    child: Option<Child>,
}

unsafe impl Send for ShmParent {}

impl ShmParent {
    pub fn new(child_path: &str, fd_send: RawFd, fd_ack: RawFd, fd_shm: RawFd, shm_size: usize) -> Self {
        Self {
            child_path: child_path.to_string(),
            fd_send,
            fd_ack,
            fd_shm,
            file_send: None,
            file_ack: None,
            shm_file: None,
            shm_ptr: ptr::null_mut(),
            shm_size,
            child: None,
        }
    }

    pub fn start(&mut self) -> std::io::Result<()> {
        // 1. Create eventfds
        let efd_send = EventFd::from_value_and_flags(0, EfdFlags::empty())?;
        let efd_ack = EventFd::from_value_and_flags(0, EfdFlags::empty())?;

        // 2. Create Memfd (SHM)
        let name = CString::new("efdstream_shm").unwrap();
        let memfd = memfd_create(name.as_c_str(), MFdFlags::empty())
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
        
        // Set size
        ftruncate(&memfd, self.shm_size as i64)
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;

        // Mmap
        let ptr = unsafe {
            mmap(
                None,
                std::num::NonZeroUsize::new(self.shm_size).unwrap(),
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_SHARED,
                &memfd,
                0,
            ).map_err(|e| std::io::Error::from_raw_os_error(e as i32))?
        };
        self.shm_ptr = ptr.as_ptr() as *mut u8;

        let raw_send = efd_send.as_raw_fd();
        let raw_ack = efd_ack.as_raw_fd();
        let raw_shm = memfd.as_raw_fd();

        // 3. Start Child
        let mut cmd = Command::new(&self.child_path);
        cmd.arg("-mode").arg("child");
        cmd.arg("-fd-send").arg(self.fd_send.to_string());
        cmd.arg("-fd-ack").arg(self.fd_ack.to_string());
        cmd.arg("-fd-shm").arg(self.fd_shm.to_string());
        cmd.arg("-shm-size").arg(self.shm_size.to_string());
        cmd.stdin(Stdio::inherit());
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());

        let target_fd_send = self.fd_send;
        let target_fd_ack = self.fd_ack;
        let target_fd_shm = self.fd_shm;

        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(raw_send, target_fd_send) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::dup2(raw_ack, target_fd_ack) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::dup2(raw_shm, target_fd_shm) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = cmd.spawn()?;
        self.child = Some(child);

        // 4. Wrap FDs
        let owned_send: OwnedFd = efd_send.into();
        let owned_ack: OwnedFd = efd_ack.into();
        let owned_shm: OwnedFd = memfd.into();
        
        self.file_send = Some(File::from(owned_send));
        self.file_ack = Some(File::from(owned_ack));
        self.shm_file = Some(File::from(owned_shm));

        Ok(())
    }

    pub fn send_data(&mut self, data: &[u8]) -> std::io::Result<()> {
        if data.len() > self.shm_size {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Data too large for SHM"));
        }

        if self.shm_ptr.is_null() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Not started"));
        }

        // Write to SHM
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), self.shm_ptr, data.len());
        }

        // Send Length via EventFD
        if let Some(file_send) = &mut self.file_send {
            let len = data.len() as u64;
            let bytes = len.to_ne_bytes();
            file_send.write_all(&bytes)?;
        } else {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Not started"));
        }

        // Wait for ACK
        if let Some(file_ack) = &mut self.file_ack {
            let mut buf = [0u8; 8];
            file_ack.read_exact(&mut buf)?;
            // Check ACK value if needed
        } else {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Not started"));
        }

        Ok(())
    }
}

impl Drop for ShmParent {
    fn drop(&mut self) {
        if !self.shm_ptr.is_null() {
            unsafe {
                if let Some(ptr) = NonNull::new(self.shm_ptr as *mut std::ffi::c_void) {
                    let _ = munmap(ptr, self.shm_size);
                }
            }
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

pub struct ShmReceiver {
    fd_send: RawFd,
    fd_ack: RawFd,
    fd_shm: RawFd,
    shm_size: usize,
    shm_ptr: *mut u8,
}

unsafe impl Send for ShmReceiver {}

impl ShmReceiver {
    pub fn new(fd_send: RawFd, fd_ack: RawFd, fd_shm: RawFd, shm_size: usize) -> Self {
        Self { fd_send, fd_ack, fd_shm, shm_size, shm_ptr: ptr::null_mut() }
    }

    pub fn listen<F>(&mut self, callback: F) -> std::io::Result<()>
    where
        F: Fn(&[u8]),
    {
        // Mmap the passed SHM FD
        // We need to wrap raw fd in BorrowedFd to satisfy AsFd trait for mmap
        let borrowed_shm = unsafe { BorrowedFd::borrow_raw(self.fd_shm) };

        let ptr = unsafe {
            mmap(
                None,
                std::num::NonZeroUsize::new(self.shm_size).unwrap(),
                ProtFlags::PROT_READ, // Read-only for receiver? Or Read-Write? Let's say Read.
                MapFlags::MAP_SHARED,
                borrowed_shm,
                0,
            ).map_err(|e| std::io::Error::from_raw_os_error(e as i32))?
        };
        self.shm_ptr = ptr.as_ptr() as *mut u8;

        let mut file_read = unsafe { File::from_raw_fd(self.fd_send) };
        let mut file_write = unsafe { File::from_raw_fd(self.fd_ack) };

        loop {
            let mut buf = [0u8; 8];
            match file_read.read_exact(&mut buf) {
                Ok(_) => {
                    let length = u64::from_ne_bytes(buf) as usize;
                    if length > self.shm_size {
                        eprintln!("Received length {} exceeds SHM size {}", length, self.shm_size);
                        continue;
                    }

                    // Read from SHM
                    let data = unsafe { slice::from_raw_parts(self.shm_ptr, length) };
                    callback(data);

                    // Send Ack (1)
                    let ack_val: u64 = 1;
                    let bytes = ack_val.to_ne_bytes();
                    file_write.write_all(&bytes)?;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for ShmReceiver {
    fn drop(&mut self) {
        if !self.shm_ptr.is_null() {
            unsafe {
                if let Some(ptr) = NonNull::new(self.shm_ptr as *mut std::ffi::c_void) {
                    let _ = munmap(ptr, self.shm_size);
                }
            }
        }
    }
}
