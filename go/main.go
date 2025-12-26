package main

import (
	"flag"
	"fmt"
	"log"
	"time"

	"github.com/snowmerak/efdstream/go/efd"
	"golang.org/x/sys/unix"
)

var (
	mode      = flag.String("mode", "parent", "Mode: parent or child")
	childPath = flag.String("child", "", "Path to child binary (required for parent mode)")
	fdSend    = flag.Int("fd-send", 3, "FD for sending data (child mode)")
	fdAck     = flag.Int("fd-ack", 4, "FD for sending ack (child mode)")
	fdShm     = flag.Int("fd-shm", 5, "FD for shared memory (child mode)")
	shmSize   = flag.Int("shm-size", 1024*1024, "Size of shared memory")
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

	// Use default FDs for child: 3, 4, 5
	parent := efd.NewShmParent(*childPath, 3, 4, 5, *shmSize)
	if err := parent.Start(); err != nil {
		log.Fatalf("Failed to start parent: %v", err)
	}
	defer parent.Close()

	fmt.Printf("[Go Parent] Child started\n")

	for i := 0; i < 5; i++ {
		msg := fmt.Sprintf("Hello from Go Parent %d", i)
		fmt.Printf("[Go Parent] Sending: %s\n", msg)

		if err := parent.SendData([]byte(msg)); err != nil {
			log.Fatalf("[Go Parent] Communication error: %v", err)
		}

		fmt.Printf("[Go Parent] Received ACK\n")
		time.Sleep(1 * time.Second)
	}
}

func runChild() {
	fmt.Printf("[Go Child] Started with fd-send=%d, fd-ack=%d, fd-shm=%d\n", *fdSend, *fdAck, *fdShm)

	// Handle FD mapping for Go Child
	// If we are running Go->Go, FDs come in at 3, 4, 5.
	// If *fdSend != 3, we dup.
	if *fdSend != 3 {
		unix.Dup2(3, *fdSend)
	}
	if *fdAck != 4 {
		unix.Dup2(4, *fdAck)
	}
	if *fdShm != 5 {
		unix.Dup2(5, *fdShm)
	}

	child := efd.NewShmChild(*fdSend, *fdAck, *fdShm, *shmSize)
	err := child.Listen(func(data []byte) {
		fmt.Printf("[Go Child] Received: %s\n", string(data))
		fmt.Printf("[Go Child] Sending ACK\n")
	})

	if err != nil {
		log.Printf("[Go Child] Error: %v", err)
	}
}
