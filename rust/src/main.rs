use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, RawFd, AsRawFd, IntoRawFd};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::os::unix::process::CommandExt;

use nix::sys::eventfd::{eventfd, EfdFlags};

fn main() {
    let args: Vec<String> = env::args().collect();
    
    let mut mode = "parent".to_string();
    let mut child_path = "".to_string();

    let mut i = 1;
    while i < args.len() {
        if args[i] == "-mode" || args[i] == "--mode" {
            if i + 1 < args.len() {
                mode = args[i+1].clone();
                i += 1;
            }
        } else if args[i] == "-child" || args[i] == "--child" {
            if i + 1 < args.len() {
                child_path = args[i+1].clone();
                i += 1;
            }
        }
        i += 1;
    }

    if mode == "parent" {
        run_parent(&child_path);
    } else {
        run_child();
    }
}

fn run_parent(child_path: &str) {
    if child_path.is_empty() {
        eprintln!("Child path is required in parent mode");
        std::process::exit(1);
    }

    // 1. Create eventfds
    // We use 0 for flags (no CLOEXEC) so they can be inherited.
    let efd_send = eventfd(0, EfdFlags::empty()).expect("Failed to create efd_send");
    let efd_ack = eventfd(0, EfdFlags::empty()).expect("Failed to create efd_ack");

    // Get raw FDs for the closure. 
    // Note: If efd_send is OwnedFd, as_raw_fd() borrows it.
    // We need to ensure the FDs are valid when the closure runs.
    let raw_send = efd_send.as_raw_fd();
    let raw_ack = efd_ack.as_raw_fd();

    println!("[Rust Parent] Created eventfds: send={}, ack={}", raw_send, raw_ack);

    // 2. Start Child
    let mut cmd = Command::new(child_path);
    cmd.arg("-mode").arg("child");
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    // Use pre_exec to set up FDs 3 and 4 in the child
    unsafe {
        cmd.pre_exec(move || {
            // FD 3 = efd_send
            if libc::dup2(raw_send, 3) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // FD 4 = efd_ack
            if libc::dup2(raw_ack, 4) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn().expect("Failed to start child");
    println!("[Rust Parent] Child started with PID {}", child.id());

    // 3. Communication Loop
    // We wrap them in File. If efd_send is OwnedFd, we should use into_raw_fd() to transfer ownership to File.
    // Or just use FromRawFd and forget the original if it was raw.
    // Since nix 0.30 eventfd returns OwnedFd, we use into_raw_fd().
    let mut file_send = unsafe { File::from_raw_fd(efd_send.into_raw_fd()) };
    let mut file_ack = unsafe { File::from_raw_fd(efd_ack.into_raw_fd()) };

    for _ in 0..5 {
        // Send Length (8 bytes)
        let length_to_send: u64 = 8;
        let bytes = length_to_send.to_ne_bytes();
        
        println!("[Rust Parent] Sending length: {}", length_to_send);
        file_send.write_all(&bytes).expect("Write error");

        // Wait for Ack
        println!("[Rust Parent] Waiting for ACK...");
        let mut buf = [0u8; 8];
        file_ack.read_exact(&mut buf).expect("Read error");
        let ack_val = u64::from_ne_bytes(buf);
        println!("[Rust Parent] Received ACK: {}", ack_val);

        thread::sleep(Duration::from_secs(1));
    }

    let _ = child.kill();
}

fn run_child() {
    println!("[Rust Child] Started");

    // FD 3 = efd_send (Read)
    // FD 4 = efd_ack (Write)
    let fd_read: RawFd = 3;
    let fd_write: RawFd = 4;

    let mut file_read = unsafe { File::from_raw_fd(fd_read) };
    let mut file_write = unsafe { File::from_raw_fd(fd_write) };

    loop {
        let mut buf = [0u8; 8];
        match file_read.read_exact(&mut buf) {
            Ok(_) => {
                let length = u64::from_ne_bytes(buf);
                println!("[Rust Child] Received length: {}", length);

                // Send Ack (1)
                let ack_val: u64 = 1;
                let bytes = ack_val.to_ne_bytes();
                file_write.write_all(&bytes).expect("Write error");
                println!("[Rust Child] Sent ACK: {}", ack_val);
            }
            Err(e) => {
                println!("[Rust Child] Read error (parent likely closed): {}", e);
                break;
            }
        }
    }
}
