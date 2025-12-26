use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, RawFd, AsRawFd};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::os::unix::process::CommandExt;

use nix::sys::eventfd::{EventFd, EfdFlags};

fn main() {
    let args: Vec<String> = env::args().collect();
    
    let mut mode = "parent".to_string();
    let mut child_path = "".to_string();
    let mut fd_send = 3;
    let mut fd_ack = 4;

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
        } else if args[i] == "-fd-send" || args[i] == "--fd-send" {
            if i + 1 < args.len() {
                fd_send = args[i+1].parse().unwrap_or(3);
                i += 1;
            }
        } else if args[i] == "-fd-ack" || args[i] == "--fd-ack" {
            if i + 1 < args.len() {
                fd_ack = args[i+1].parse().unwrap_or(4);
                i += 1;
            }
        }
        i += 1;
    }

    if mode == "parent" {
        run_parent(&child_path, fd_send, fd_ack);
    } else {
        run_child(fd_send, fd_ack);
    }
}

fn run_parent(child_path: &str, target_fd_send: i32, target_fd_ack: i32) {
    if child_path.is_empty() {
        eprintln!("Child path is required in parent mode");
        std::process::exit(1);
    }

    // 1. Create eventfds
    // We use 0 for flags (no CLOEXEC) so they can be inherited.
    let efd_send = EventFd::from_value_and_flags(0, EfdFlags::empty()).expect("Failed to create efd_send");
    let efd_ack = EventFd::from_value_and_flags(0, EfdFlags::empty()).expect("Failed to create efd_ack");

    // Get raw FDs for the closure. 
    let raw_send = efd_send.as_raw_fd();
    let raw_ack = efd_ack.as_raw_fd();

    println!("[Rust Parent] Created eventfds: send={}, ack={}", raw_send, raw_ack);

    // 2. Start Child
    let mut cmd = Command::new(child_path);
    cmd.arg("-mode").arg("child");
    cmd.arg("-fd-send").arg(target_fd_send.to_string());
    cmd.arg("-fd-ack").arg(target_fd_ack.to_string());
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    // Use pre_exec to set up FDs in the child
    unsafe {
        cmd.pre_exec(move || {
            // Map efd_send to target_fd_send
            if libc::dup2(raw_send, target_fd_send) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // Map efd_ack to target_fd_ack
            if libc::dup2(raw_ack, target_fd_ack) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn().expect("Failed to start child");
    println!("[Rust Parent] Child started with PID {}", child.id());

    // 3. Communication Loop
    // We wrap them in File.
    // EventFd implements AsRawFd and IntoRawFd (via From<EventFd> for OwnedFd).
    // But nix 0.30 EventFd might not have into_raw_fd directly if it wraps OwnedFd internally or similar.
    // Let's use as_raw_fd() and std::mem::forget or similar if we want to transfer ownership, 
    // OR just use FromRawFd and let File take ownership (and close it).
    // If we use as_raw_fd(), File will close the FD on drop. EventFd will ALSO close it on drop. Double close!
    // We need to consume efd_send.
    // nix::sys::eventfd::EventFd implements Into<OwnedFd>.
    use std::os::unix::io::OwnedFd;
    let owned_send: OwnedFd = efd_send.into();
    let owned_ack: OwnedFd = efd_ack.into();
    let mut file_send = File::from(owned_send);
    let mut file_ack = File::from(owned_ack);

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

fn run_child(fd_send: RawFd, fd_ack: RawFd) {
    println!("[Rust Child] Started with fd_send={}, fd_ack={}", fd_send, fd_ack);

    let mut file_read = unsafe { File::from_raw_fd(fd_send) };
    let mut file_write = unsafe { File::from_raw_fd(fd_ack) };

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
