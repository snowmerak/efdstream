use std::env;
use std::thread;
use std::time::Duration;

use efdstream::{ShmParent, ShmReceiver};

fn main() {
    let args: Vec<String> = env::args().collect();
    
    let mut mode = "parent".to_string();
    let mut child_path = "".to_string();
    let mut fd_send = 3;
    let mut fd_ack = 4;
    let mut fd_shm = 5;
    let mut shm_size = 1024 * 1024;

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
        } else if args[i] == "-fd-shm" || args[i] == "--fd-shm" {
            if i + 1 < args.len() {
                fd_shm = args[i+1].parse().unwrap_or(5);
                i += 1;
            }
        } else if args[i] == "-shm-size" || args[i] == "--shm-size" {
            if i + 1 < args.len() {
                shm_size = args[i+1].parse().unwrap_or(1024 * 1024);
                i += 1;
            }
        }
        i += 1;
    }

    if mode == "parent" {
        run_parent(&child_path, fd_send, fd_ack, fd_shm, shm_size);
    } else {
        run_child(fd_send, fd_ack, fd_shm, shm_size);
    }
}

fn run_parent(child_path: &str, fd_send: i32, fd_ack: i32, fd_shm: i32, shm_size: usize) {
    if child_path.is_empty() {
        eprintln!("Child path is required in parent mode");
        std::process::exit(1);
    }

    let mut parent = ShmParent::new(child_path, fd_send, fd_ack, fd_shm, shm_size);
    parent.start().expect("Failed to start parent");

    println!("[Rust Parent] Child started");

    for i in 0..5 {
        let msg = format!("Hello from Rust Parent {}", i);
        println!("[Rust Parent] Sending: {}", msg);
        
        parent.send_data(msg.as_bytes()).expect("Communication error");

        println!("[Rust Parent] Received ACK");
        thread::sleep(Duration::from_secs(1));
    }
}

fn run_child(fd_send: i32, fd_ack: i32, fd_shm: i32, shm_size: usize) {
    println!("[Rust Child] Started with fd_send={}, fd_ack={}, fd_shm={}", fd_send, fd_ack, fd_shm);

    let mut receiver = ShmReceiver::new(fd_send, fd_ack, fd_shm, shm_size);
    let res = receiver.listen(|data| {
        let msg = String::from_utf8_lossy(data);
        println!("[Rust Child] Received: {}", msg);
        println!("[Rust Child] Sending ACK");
    });

    if let Err(e) = res {
        println!("[Rust Child] Error: {}", e);
    }
}
