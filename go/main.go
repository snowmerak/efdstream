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
	fdSend    = flag.Int("fd-send", 3, "FD for sending data (child mode)")
	fdAck     = flag.Int("fd-ack", 4, "FD for sending ack (child mode)")
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
	// We pass the FD numbers we intend the child to use.
	// Since we are using ExtraFiles, the first file will be FD 3, the second FD 4.
	// So we tell the child to use 3 and 4.
	// If we wanted to be dynamic, we could change the order or add dummy files,
	// but ExtraFiles always maps sequentially starting from 3.
	targetFdSend := 3
	targetFdAck := 4

	cmd := exec.Command(*childPath,
		"-mode", "child",
		"-fd-send", fmt.Sprintf("%d", targetFdSend),
		"-fd-ack", fmt.Sprintf("%d", targetFdAck),
	)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr

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
	native := binary.NativeEndian

	for i := 0; i < 5; i++ {
		var lengthToSend uint64 = 8
		native.PutUint64(buf, lengthToSend)

		fmt.Printf("[Go Parent] Sending length: %d\n", lengthToSend)
		_, err := unix.Write(efdSend, buf)
		if err != nil {
			log.Fatalf("[Go Parent] Write error: %v", err)
		}

		fmt.Printf("[Go Parent] Waiting for ACK...\n")
		_, err = unix.Read(efdAck, buf)
		if err != nil {
			log.Fatalf("[Go Parent] Read error: %v", err)
		}
		ackVal := native.Uint64(buf)
		fmt.Printf("[Go Parent] Received ACK: %d\n", ackVal)

		time.Sleep(1 * time.Second)
	}

	cmd.Process.Kill()
}

func runChild() {
	fmt.Printf("[Go Child] Started with fd-send=%d, fd-ack=%d\n", *fdSend, *fdAck)

	// Use the FDs passed via flags
	fdRead := *fdSend
	fdWrite := *fdAck

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
		var ackVal uint64 = 1
		native.PutUint64(buf, ackVal)

		_, err = unix.Write(fdWrite, buf)
		if err != nil {
			log.Fatalf("[Go Child] Write error: %v", err)
		}
		fmt.Printf("[Go Child] Sent ACK: %d\n", ackVal)
	}
}
