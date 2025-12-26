package main

import (
	"encoding/binary"
	"flag"
	"fmt"
	"log"
	"os"
	"os/exec"
	"time"

	"golang.org/x/sys/unix"
)

var (
	mode      = flag.String("mode", "parent", "Mode: parent or child")
	childPath = flag.String("child", "", "Path to child binary (required for parent mode)")
)

func main() {
	flag.Parse()

	if *mode == "parent" {
		runParent()
	} else {
		runChild()
	}
}

func runParent() {
	if *childPath == "" {
		log.Fatal("Child path is required in parent mode")
	}

	// 1. Create eventfds
	// EFD_CLOEXEC is usually good practice, but we want to pass them to child.
	// However, Go's os.File handling usually sets CLOEXEC by default and clears it for ExtraFiles.
	// Let's use 0 for flags.
	efdSend, err := unix.Eventfd(0, 0)
	if err != nil {
		log.Fatalf("Failed to create efdSend: %v", err)
	}
	efdAck, err := unix.Eventfd(0, 0)
	if err != nil {
		log.Fatalf("Failed to create efdAck: %v", err)
	}

	// Wrap in os.File to pass to ExtraFiles
	fileSend := os.NewFile(uintptr(efdSend), "efd_send")
	fileAck := os.NewFile(uintptr(efdAck), "efd_ack")
	defer fileSend.Close()
	defer fileAck.Close()

	// 2. Start Child
	cmd := exec.Command(*childPath, "-mode", "child")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	// FD 0, 1, 2 are inherited by default if Stdin/out/err are set or nil (depending on setup).
	// ExtraFiles starts at FD 3.
	// FD 3 = efdSend (Parent writes, Child reads)
	// FD 4 = efdAck (Parent reads, Child writes)
	cmd.ExtraFiles = []*os.File{fileSend, fileAck}

	if err := cmd.Start(); err != nil {
		log.Fatalf("Failed to start child: %v", err)
	}
	fmt.Printf("[Go Parent] Child started with PID %d\n", cmd.Process.Pid)

	// 3. Communication Loop
	buf := make([]byte, 8)
	// Native endian for eventfd
	native := binary.NativeEndian

	for i := 0; i < 5; i++ {
		// Send Length (e.g., 8 bytes)
		// In eventfd, write adds to the counter.
		// The user said "Send length". Let's send the value '8'.
		var lengthToSend uint64 = 8
		native.PutUint64(buf, lengthToSend)

		fmt.Printf("[Go Parent] Sending length: %d\n", lengthToSend)
		_, err := unix.Write(efdSend, buf)
		if err != nil {
			log.Fatalf("[Go Parent] Write error: %v", err)
		}

		// Wait for Ack
		// User wanted -1, but eventfd doesn't support it. We expect 1 (or agreed value).
		fmt.Printf("[Go Parent] Waiting for ACK...\n")
		_, err = unix.Read(efdAck, buf)
		if err != nil {
			log.Fatalf("[Go Parent] Read error: %v", err)
		}
		ackVal := native.Uint64(buf)
		fmt.Printf("[Go Parent] Received ACK: %d\n", ackVal)

		time.Sleep(1 * time.Second)
	}

	// Cleanup
	cmd.Process.Kill()
}

func runChild() {
	fmt.Println("[Go Child] Started")

	// FD 3 = efdSend (Read)
	// FD 4 = efdAck (Write)
	// In Go, we can construct os.File from uintptr(3) and uintptr(4)
	// But we need to be careful about validity.

	// Note: os.NewFile returns nil if fd is invalid, but here we assume they are passed.
	// We use unix.Read/Write directly on the FDs.
	fdRead := 3
	fdWrite := 4

	buf := make([]byte, 8)
	native := binary.NativeEndian

	for {
		// Read Length
		_, err := unix.Read(fdRead, buf)
		if err != nil {
			log.Printf("[Go Child] Read error (parent likely closed): %v", err)
			break
		}
		length := native.Uint64(buf)
		fmt.Printf("[Go Child] Received length: %d\n", length)

		// Send Ack
		// User requested -1, but we use 1.
		// Or 0xFFFFFFFFFFFFFFFE if we want to simulate a large unsigned number close to -1.
		// Let's use 1 as agreed.
		var ackVal uint64 = 1
		native.PutUint64(buf, ackVal)

		_, err = unix.Write(fdWrite, buf)
		if err != nil {
			log.Fatalf("[Go Child] Write error: %v", err)
		}
		fmt.Printf("[Go Child] Sent ACK: %d\n", ackVal)
	}
}
