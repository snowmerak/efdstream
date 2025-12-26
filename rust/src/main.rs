use std::env;
use std::thread;
use std::time::Duration;

use efdstream::{ShmParent, ShmChild};

fn main() {
    let args: Vec<String> = env::args().collect();
    
    let mut mode = "parent".to_string();
    let mut child_path = "".to_string();
    
    let mut fd_p2c_send = 3;
    let mut fd_p2c_ack = 4;
    let mut fd_p2c_shm = 5;
    let mut fd_c2p_send = 6;
    let mut fd_c2p_ack = 7;
    let mut fd_c2p_shm = 8;

    let mut shm_size = 1024 * 1024;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "-mode" || args[i] == "--mode" {
            if i + 1 < args.len() { mode = args[i+1].clone(); i += 1; }
        } else if args[i] == "-child" || args[i] == "--child" {
            if i + 1 < args.len() { child_path = args[i+1].clone(); i += 1; }
        } else if args[i] == "-fd-p2c-send" {
            if i + 1 < args.len() { fd_p2c_send = args[i+1].parse().unwrap_or(3); i += 1; }
        } else if args[i] == "-fd-p2c-ack" {
            if i + 1 < args.len() { fd_p2c_ack = args[i+1].parse().unwrap_or(4); i += 1; }
        } else if args[i] == "-fd-p2c-shm" {
            if i + 1 < args.len() { fd_p2c_shm = args[i+1].parse().unwrap_or(5); i += 1; }
        } else if args[i] == "-fd-c2p-send" {
            if i + 1 < args.len() { fd_c2p_send = args[i+1].parse().unwrap_or(6); i += 1; }
        } else if args[i] == "-fd-c2p-ack" {
            if i + 1 < args.len() { fd_c2p_ack = args[i+1].parse().unwrap_or(7); i += 1; }
        } else if args[i] == "-fd-c2p-shm" {
            if i + 1 < args.len() { fd_c2p_shm = args[i+1].parse().unwrap_or(8); i += 1; }
        } else if args[i] == "-shm-size" || args[i] == "--shm-size" {
            if i + 1 < args.len() { shm_size = args[i+1].parse().unwrap_or(1024 * 1024); i += 1; }
        }
        i += 1;
    }

    if mode == "parent" {
        run_parent(&child_path, fd_p2c_send, fd_p2c_ack, fd_p2c_shm, fd_c2p_send, fd_c2p_ack, fd_c2p_shm, shm_size);
    } else {
        run_child(fd_p2c_send, fd_p2c_ack, fd_p2c_shm, fd_c2p_send, fd_c2p_ack, fd_c2p_shm, shm_size);
    }
}

fn run_parent(child_path: &str, 
              fd_p2c_send: i32, fd_p2c_ack: i32, fd_p2c_shm: i32,
              fd_c2p_send: i32, fd_c2p_ack: i32, fd_c2p_shm: i32,
              shm_size: usize) {
    if child_path.is_empty() {
        eprintln!("Child path is required in parent mode");
        std::process::exit(1);
    }

    let mut parent = ShmParent::new(child_path, 
        fd_p2c_send, fd_p2c_ack, fd_p2c_shm,
        fd_c2p_send, fd_c2p_ack, fd_c2p_shm,
        shm_size);
    parent.start().expect("Failed to start parent");

    println!("[Rust Parent] Child started");

    for i in 0..5 {
        // Send
        let msg = format!("Hello from Rust Parent {}", i);
        println!("[Rust Parent] Sending: {}", msg);
        parent.send_data(msg.as_bytes()).expect("Communication error");
        println!("[Rust Parent] Received ACK");

        // Receive
        match parent.read_data() {
            Ok(data) => {
                let msg = String::from_utf8_lossy(&data);
                println!("[Rust Parent] Received: {}", msg);
            },
            Err(e) => println!("[Rust Parent] Read error: {}", e),
        }

        thread::sleep(Duration::from_secs(1));
    }
}

fn run_child(fd_p2c_send: i32, fd_p2c_ack: i32, fd_p2c_shm: i32,
             fd_c2p_send: i32, fd_c2p_ack: i32, fd_c2p_shm: i32,
             shm_size: usize) {
    println!("[Rust Child] Started with P2C({},{},{}) C2P({},{},{})", 
        fd_p2c_send, fd_p2c_ack, fd_p2c_shm, fd_c2p_send, fd_c2p_ack, fd_c2p_shm);

    let mut child = ShmChild::new(
        fd_p2c_send, fd_p2c_ack, fd_p2c_shm,
        fd_c2p_send, fd_c2p_ack, fd_c2p_shm,
        shm_size);

    // Spawn thread to send data back
    let mut child_sender = ShmChild::new(
        fd_p2c_send, fd_p2c_ack, fd_p2c_shm,
        fd_c2p_send, fd_c2p_ack, fd_c2p_shm,
        shm_size);
    
    thread::spawn(move || {
        for i in 0..5 {
            thread::sleep(Duration::from_millis(500));
            let msg = format!("Hello from Rust Child {}", i);
            println!("[Rust Child] Sending: {}", msg);
            if let Err(e) = child_sender.send_data(msg.as_bytes()) {
                println!("[Rust Child] Send error: {}", e);
                break;
            }
        }
    });

    let res = child.listen(|data| {
        let msg = String::from_utf8_lossy(data);
        println!("[Rust Child] Received: {}", msg);
        println!("[Rust Child] Sending ACK");
    });

    if let Err(e) = res {
        println!("[Rust Child] Error: {}", e);
    }
}
