use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd, BorrowedFd};
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
    shm_size: usize,

    // Resources
    file_p2c_send: Option<File>,
    file_p2c_ack: Option<File>,
    shm_p2c_file: Option<File>,
    shm_p2c_ptr: *mut u8,

    file_c2p_send: Option<File>,
    file_c2p_ack: Option<File>,
    shm_c2p_file: Option<File>,
    shm_c2p_ptr: *mut u8,

    child: Option<Child>,
}

unsafe impl Send for ShmParent {}

impl ShmParent {
    pub fn new(child_path: &str, shm_size: usize) -> Self {
        Self {
            child_path: child_path.to_string(),
            shm_size,
            file_p2c_send: None, file_p2c_ack: None, shm_p2c_file: None, shm_p2c_ptr: ptr::null_mut(),
            file_c2p_send: None, file_c2p_ack: None, shm_c2p_file: None, shm_c2p_ptr: ptr::null_mut(),
            child: None,
        }
    }

    pub fn start(&mut self) -> std::io::Result<()> {
        // 1. Create P2C resources
        let efd_p2c_send = EventFd::from_value_and_flags(0, EfdFlags::empty())?;
        let efd_p2c_ack = EventFd::from_value_and_flags(0, EfdFlags::empty())?;
        let name_p2c = CString::new("efdstream_shm_p2c").unwrap();
        let memfd_p2c = memfd_create(name_p2c.as_c_str(), MFdFlags::empty())
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
        ftruncate(&memfd_p2c, self.shm_size as i64)
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
        let ptr_p2c = unsafe {
            mmap(None, std::num::NonZeroUsize::new(self.shm_size).unwrap(),
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE, MapFlags::MAP_SHARED, &memfd_p2c, 0)
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?
        };
        self.shm_p2c_ptr = ptr_p2c.as_ptr() as *mut u8;

        // 2. Create C2P resources
        let efd_c2p_send = EventFd::from_value_and_flags(0, EfdFlags::empty())?;
        let efd_c2p_ack = EventFd::from_value_and_flags(0, EfdFlags::empty())?;
        let name_c2p = CString::new("efdstream_shm_c2p").unwrap();
        let memfd_c2p = memfd_create(name_c2p.as_c_str(), MFdFlags::empty())
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
        ftruncate(&memfd_c2p, self.shm_size as i64)
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
        let ptr_c2p = unsafe {
            mmap(None, std::num::NonZeroUsize::new(self.shm_size).unwrap(),
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE, MapFlags::MAP_SHARED, &memfd_c2p, 0)
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?
        };
        self.shm_c2p_ptr = ptr_c2p.as_ptr() as *mut u8;

        // Raw FDs for dup2
        let raw_p2c_send = efd_p2c_send.as_raw_fd();
        let raw_p2c_ack = efd_p2c_ack.as_raw_fd();
        let raw_p2c_shm = memfd_p2c.as_raw_fd();
        let raw_c2p_send = efd_c2p_send.as_raw_fd();
        let raw_c2p_ack = efd_c2p_ack.as_raw_fd();
        let raw_c2p_shm = memfd_c2p.as_raw_fd();

        // 3. Start Child
        let mut cmd = Command::new(&self.child_path);
        cmd.arg("-mode").arg("child");
        // We map the FDs to 3, 4, 5, 6, 7, 8 in the child process.
        cmd.arg("-fd-p2c-send").arg("3");
        cmd.arg("-fd-p2c-ack").arg("4");
        cmd.arg("-fd-p2c-shm").arg("5");
        cmd.arg("-fd-c2p-send").arg("6");
        cmd.arg("-fd-c2p-ack").arg("7");
        cmd.arg("-fd-c2p-shm").arg("8");
        cmd.arg("-shm-size").arg(self.shm_size.to_string());
        cmd.stdin(Stdio::inherit());
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());

        let target_p2c_send = 3;
        let target_p2c_ack = 4;
        let target_p2c_shm = 5;
        let target_c2p_send = 6;
        let target_c2p_ack = 7;
        let target_c2p_shm = 8;

        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(raw_p2c_send, target_p2c_send) == -1 { return Err(std::io::Error::last_os_error()); }
                if libc::dup2(raw_p2c_ack, target_p2c_ack) == -1 { return Err(std::io::Error::last_os_error()); }
                if libc::dup2(raw_p2c_shm, target_p2c_shm) == -1 { return Err(std::io::Error::last_os_error()); }
                if libc::dup2(raw_c2p_send, target_c2p_send) == -1 { return Err(std::io::Error::last_os_error()); }
                if libc::dup2(raw_c2p_ack, target_c2p_ack) == -1 { return Err(std::io::Error::last_os_error()); }
                if libc::dup2(raw_c2p_shm, target_c2p_shm) == -1 { return Err(std::io::Error::last_os_error()); }
                Ok(())
            });
        }

        let child = cmd.spawn()?;
        self.child = Some(child);

        // 4. Wrap FDs
        self.file_p2c_send = Some(File::from(OwnedFd::from(efd_p2c_send)));
        self.file_p2c_ack = Some(File::from(OwnedFd::from(efd_p2c_ack)));
        self.shm_p2c_file = Some(File::from(OwnedFd::from(memfd_p2c)));

        self.file_c2p_send = Some(File::from(OwnedFd::from(efd_c2p_send)));
        self.file_c2p_ack = Some(File::from(OwnedFd::from(efd_c2p_ack)));
        self.shm_c2p_file = Some(File::from(OwnedFd::from(memfd_c2p)));

        Ok(())
    }

    pub fn send_data(&mut self, data: &[u8]) -> std::io::Result<()> {
        if data.len() > self.shm_size {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Data too large for SHM"));
        }
        if self.shm_p2c_ptr.is_null() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Not started"));
        }

        // Write to SHM
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), self.shm_p2c_ptr, data.len());
        }

        // Send Length
        if let Some(file_send) = &mut self.file_p2c_send {
            let len = data.len() as u64;
            let bytes = len.to_ne_bytes();
            file_send.write_all(&bytes)?;
        }

        // Wait for ACK
        if let Some(file_ack) = &mut self.file_p2c_ack {
            let mut buf = [0u8; 8];
            file_ack.read_exact(&mut buf)?;
        }

        Ok(())
    }

    pub fn read_data(&mut self) -> std::io::Result<Vec<u8>> {
        if self.shm_c2p_ptr.is_null() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Not started"));
        }

        // Wait for Signal
        let length = if let Some(file_read) = &mut self.file_c2p_send {
            let mut buf = [0u8; 8];
            file_read.read_exact(&mut buf)?;
            u64::from_ne_bytes(buf) as usize
        } else {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Not started"));
        };

        if length > self.shm_size {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Received length exceeds SHM size"));
        }

        // Read from SHM
        let data = unsafe { slice::from_raw_parts(self.shm_c2p_ptr, length).to_vec() };

        // Send ACK
        if let Some(file_write) = &mut self.file_c2p_ack {
            let ack_val: u64 = 1;
            let bytes = ack_val.to_ne_bytes();
            file_write.write_all(&bytes)?;
        }

        Ok(data)
    }
}

impl Drop for ShmParent {
    fn drop(&mut self) {
        if !self.shm_p2c_ptr.is_null() {
            unsafe {
                if let Some(ptr) = NonNull::new(self.shm_p2c_ptr as *mut std::ffi::c_void) {
                    let _ = munmap(ptr, self.shm_size);
                }
            }
        }
        if !self.shm_c2p_ptr.is_null() {
            unsafe {
                if let Some(ptr) = NonNull::new(self.shm_c2p_ptr as *mut std::ffi::c_void) {
                    let _ = munmap(ptr, self.shm_size);
                }
            }
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

pub struct ShmChild {
    fd_p2c_send: RawFd,
    fd_p2c_ack: RawFd,
    fd_p2c_shm: RawFd,
    fd_c2p_send: RawFd,
    fd_c2p_ack: RawFd,
    fd_c2p_shm: RawFd,
    shm_size: usize,
    shm_p2c_ptr: *mut u8,
    shm_c2p_ptr: *mut u8,
}

unsafe impl Send for ShmChild {}

impl ShmChild {
    pub fn new(fd_p2c_send: RawFd, fd_p2c_ack: RawFd, fd_p2c_shm: RawFd,
               fd_c2p_send: RawFd, fd_c2p_ack: RawFd, fd_c2p_shm: RawFd,
               shm_size: usize) -> Self {
        Self { 
            fd_p2c_send, fd_p2c_ack, fd_p2c_shm,
            fd_c2p_send, fd_c2p_ack, fd_c2p_shm,
            shm_size, 
            shm_p2c_ptr: ptr::null_mut(),
            shm_c2p_ptr: ptr::null_mut(),
        }
    }

    pub fn init(&mut self) -> std::io::Result<()> {
        // Mmap P2C (Read)
        let borrowed_p2c = unsafe { BorrowedFd::borrow_raw(self.fd_p2c_shm) };
        let ptr_p2c = unsafe {
            mmap(None, std::num::NonZeroUsize::new(self.shm_size).unwrap(),
                ProtFlags::PROT_READ, MapFlags::MAP_SHARED, borrowed_p2c, 0)
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?
        };
        self.shm_p2c_ptr = ptr_p2c.as_ptr() as *mut u8;

        // Mmap C2P (Write)
        let borrowed_c2p = unsafe { BorrowedFd::borrow_raw(self.fd_c2p_shm) };
        let ptr_c2p = unsafe {
            mmap(None, std::num::NonZeroUsize::new(self.shm_size).unwrap(),
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE, MapFlags::MAP_SHARED, borrowed_c2p, 0)
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?
        };
        self.shm_c2p_ptr = ptr_c2p.as_ptr() as *mut u8;

        Ok(())
    }

    pub fn listen<F>(&mut self, callback: F) -> std::io::Result<()>
    where
        F: Fn(&[u8]),
    {
        if self.shm_p2c_ptr.is_null() {
            self.init()?;
        }

        let mut file_read = unsafe { File::from_raw_fd(self.fd_p2c_send) };
        let mut file_write = unsafe { File::from_raw_fd(self.fd_p2c_ack) };

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
                    let data = unsafe { slice::from_raw_parts(self.shm_p2c_ptr, length) };
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

    pub fn send_data(&mut self, data: &[u8]) -> std::io::Result<()> {
        if self.shm_c2p_ptr.is_null() {
            self.init()?;
        }
        if data.len() > self.shm_size {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Data too large for SHM"));
        }

        // Write to SHM
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), self.shm_c2p_ptr, data.len());
        }

        // Send Length
        let mut file_send = unsafe { File::from_raw_fd(self.fd_c2p_send) };
        let len = data.len() as u64;
        let bytes = len.to_ne_bytes();
        file_send.write_all(&bytes)?;
        // Prevent closing fd when file_send drops
        let _ = file_send.into_raw_fd();

        // Wait for ACK
        let mut file_ack = unsafe { File::from_raw_fd(self.fd_c2p_ack) };
        let mut buf = [0u8; 8];
        file_ack.read_exact(&mut buf)?;
        let _ = file_ack.into_raw_fd();

        Ok(())
    }
}

impl Drop for ShmChild {
    fn drop(&mut self) {
        if !self.shm_p2c_ptr.is_null() {
            unsafe {
                if let Some(ptr) = NonNull::new(self.shm_p2c_ptr as *mut std::ffi::c_void) {
                    let _ = munmap(ptr, self.shm_size);
                }
            }
        }
        if !self.shm_c2p_ptr.is_null() {
            unsafe {
                if let Some(ptr) = NonNull::new(self.shm_c2p_ptr as *mut std::ffi::c_void) {
                    let _ = munmap(ptr, self.shm_size);
                }
            }
        }
    }
}
