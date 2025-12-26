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

	fdP2CSend = flag.Int("fd-p2c-send", 3, "FD for P2C send")
	fdP2CAck  = flag.Int("fd-p2c-ack", 4, "FD for P2C ack")
	fdP2CShm  = flag.Int("fd-p2c-shm", 5, "FD for P2C shm")

	fdC2PSend = flag.Int("fd-c2p-send", 6, "FD for C2P send")
	fdC2PAck  = flag.Int("fd-c2p-ack", 7, "FD for C2P ack")
	fdC2PShm  = flag.Int("fd-c2p-shm", 8, "FD for C2P shm")

	shmSize = flag.Int("shm-size", 1024*1024, "Size of shared memory")
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

	// FDs are now auto-generated and mapped to 3, 4, 5, 6, 7, 8 in the child.
	parent := efd.NewShmParent(*childPath, *shmSize)
	if err := parent.Start(); err != nil {
		log.Fatalf("Failed to start parent: %v", err)
	}
	defer parent.Close()

	fmt.Printf("[Go Parent] Child started\n")

	for i := 0; i < 5; i++ {
		// Send
		msg := fmt.Sprintf("Hello from Go Parent %d", i)
		fmt.Printf("[Go Parent] Sending: %s\n", msg)
		if err := parent.SendData([]byte(msg)); err != nil {
			log.Fatalf("[Go Parent] Send error: %v", err)
		}
		fmt.Printf("[Go Parent] Received ACK\n")

		// Receive
		data, err := parent.ReadData()
		if err != nil {
			log.Fatalf("[Go Parent] Read error: %v", err)
		}
		fmt.Printf("[Go Parent] Received: %s\n", string(data))

		time.Sleep(1 * time.Second)
	}
}

func runChild() {
	fmt.Printf("[Go Child] Started with FDs: P2C(%d,%d,%d) C2P(%d,%d,%d)\n",
		*fdP2CSend, *fdP2CAck, *fdP2CShm, *fdC2PSend, *fdC2PAck, *fdC2PShm)

	// Handle FD mapping for Go Child
	// FDs come in at 3, 4, 5, 6, 7, 8
	// If flags differ, we dup.
	if *fdP2CSend != 3 {
		unix.Dup2(3, *fdP2CSend)
	}
	if *fdP2CAck != 4 {
		unix.Dup2(4, *fdP2CAck)
	}
	if *fdP2CShm != 5 {
		unix.Dup2(5, *fdP2CShm)
	}
	if *fdC2PSend != 6 {
		unix.Dup2(6, *fdC2PSend)
	}
	if *fdC2PAck != 7 {
		unix.Dup2(7, *fdC2PAck)
	}
	if *fdC2PShm != 8 {
		unix.Dup2(8, *fdC2PShm)
	}

	child, err := efd.NewShmChild(*fdP2CSend, *fdP2CAck, *fdP2CShm, *fdC2PSend, *fdC2PAck, *fdC2PShm, *shmSize)
	if err != nil {
		log.Fatalf("[Go Child] Failed to create child: %v", err)
	}
	defer child.Close()

	// Start a goroutine to send data back
	go func() {
		for i := 0; i < 5; i++ {
			time.Sleep(500 * time.Millisecond) // Wait a bit
			msg := fmt.Sprintf("Hello from Go Child %d", i)
			fmt.Printf("[Go Child] Sending: %s\n", msg)
			if err := child.SendData([]byte(msg)); err != nil {
				log.Printf("[Go Child] Send error: %v", err)
				return
			}
		}
	}()

	err = child.Listen(func(data []byte) {
		fmt.Printf("[Go Child] Received: %s\n", string(data))
		fmt.Printf("[Go Child] Sending ACK\n")
	})

	if err != nil {
		log.Printf("[Go Child] Error: %v", err)
	}
}
