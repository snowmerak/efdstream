# efdstream

**efdstream** is a high-performance, bidirectional Inter-Process Communication (IPC) library for Linux, designed to bridge **Go** and **Rust** applications.

It leverages `eventfd` for lightweight signaling and `memfd` (Shared Memory) for efficient data transfer, minimizing overhead and copying.

## Features

- **High Performance**: Uses Shared Memory (`memfd_create` + `mmap`) for data payloads, avoiding pipe/socket buffer copying.
- **Low Latency Signaling**: Uses `eventfd` for fast notification and acknowledgment.
- **Bidirectional**: Full duplex communication support (Parent ↔ Child).
- **Cross-Language**: Seamless communication between Go and Rust processes.
    - Go Parent ↔ Rust Child
    - Rust Parent ↔ Go Child
- **Zero Dependencies (Go)**: Uses standard library + `golang.org/x/sys/unix`.
- **Minimal Dependencies (Rust)**: Uses `nix` crate for system calls.

## Architecture

The communication relies on passing File Descriptors (FDs) from the Parent process to the Child process. 6 FDs are used in total:

### Parent to Child (P2C) Channel
1.  **P2C Send (EventFD)**: Parent signals Child that data is ready (writes data length).
2.  **P2C Ack (EventFD)**: Child signals Parent that data has been read.
3.  **P2C SHM (MemFD)**: Shared memory region for Parent to write data.

### Child to Parent (C2P) Channel
4.  **C2P Send (EventFD)**: Child signals Parent that data is ready (writes data length).
5.  **C2P Ack (EventFD)**: Parent signals Child that data has been read.
6.  **C2P SHM (MemFD)**: Shared memory region for Child to write data.

## Prerequisites

- **Linux**: This library relies on Linux-specific features (`eventfd`, `memfd_create`).
- **Go**: 1.20+
- **Rust**: 1.70+

## Build

### Build Rust
```bash
cd rust
cargo build --release
```
The binary will be at `rust/target/release/efdstream`.

### Build Go
```bash
cd go
go build -o efdstream_go main.go
```
The binary will be at `go/efdstream_go`.

## Usage Examples

### 1. Go Parent ↔ Rust Child

Run the Go program as the parent, specifying the Rust binary as the child.

```bash
cd go
./efdstream_go -mode parent -child ../rust/target/release/efdstream
```

### 2. Rust Parent ↔ Go Child

Run the Rust program as the parent, specifying the Go binary as the child.

```bash
cd rust
./target/release/efdstream -mode parent -child ../go/efdstream_go
```

## Library Usage

### Go

```go
import "github.com/snowmerak/efdstream/go/efd"

// Parent
// FDs are auto-generated and mapped to 3, 4, 5, 6, 7, 8 in the child process.
parent := efd.NewShmParent("/path/to/child", 1024*1024)
parent.Start()
parent.SendData([]byte("Hello"))
data, _ := parent.ReadData()

// Child
// The child process receives FDs 3, 4, 5, 6, 7, 8.
child, _ := efd.NewShmChild(3, 4, 5, 6, 7, 8, 1024*1024)
child.Listen(func(data []byte) {
    // Handle received data
})
child.SendData([]byte("Reply"))
```

### Rust

```rust
use efdstream::{ShmParent, ShmChild};

// Parent
// FDs are auto-generated and mapped to 3, 4, 5, 6, 7, 8 in the child process.
let mut parent = ShmParent::new("/path/to/child", 1024*1024);
parent.start().unwrap();
parent.send_data(b"Hello").unwrap();
let data = parent.read_data().unwrap();

// Child
// The child process receives FDs 3, 4, 5, 6, 7, 8.
let mut child = ShmChild::new(3, 4, 5, 6, 7, 8, 1024*1024);
child.listen(|data| {
    // Handle received data
}).unwrap();
child.send_data(b"Reply").unwrap();
```

## License

MIT
