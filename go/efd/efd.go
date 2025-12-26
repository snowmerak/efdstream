package efd

import (
	"encoding/binary"
	"fmt"
	"os"
	"os/exec"

	"golang.org/x/sys/unix"
)

// ShmParent manages the child process, eventfd, and shared memory.
type ShmParent struct {
	childPath string
	fdSend    int
	fdAck     int
	fdShm     int
	shmSize   int
	efdSend   int
	efdAck    int
	memfd     int
	shmPtr    []byte
	cmd       *exec.Cmd
	fileSend  *os.File
	fileAck   *os.File
	fileShm   *os.File
}

// NewShmParent creates a new ShmParent instance.
func NewShmParent(childPath string, fdSend, fdAck, fdShm, shmSize int) *ShmParent {
	return &ShmParent{
		childPath: childPath,
		fdSend:    fdSend,
		fdAck:     fdAck,
		fdShm:     fdShm,
		shmSize:   shmSize,
	}
}

// Start launches the child process and sets up resources.
func (p *ShmParent) Start() error {
	var err error
	// 1. Create eventfds
	p.efdSend, err = unix.Eventfd(0, 0)
	if err != nil {
		return fmt.Errorf("failed to create efdSend: %w", err)
	}
	p.efdAck, err = unix.Eventfd(0, 0)
	if err != nil {
		unix.Close(p.efdSend)
		return fmt.Errorf("failed to create efdAck: %w", err)
	}

	// 2. Create Memfd (SHM)
	p.memfd, err = unix.MemfdCreate("efdstream_shm", 0)
	if err != nil {
		unix.Close(p.efdSend)
		unix.Close(p.efdAck)
		return fmt.Errorf("failed to create memfd: %w", err)
	}

	// Set size
	if err := unix.Ftruncate(p.memfd, int64(p.shmSize)); err != nil {
		unix.Close(p.efdSend)
		unix.Close(p.efdAck)
		unix.Close(p.memfd)
		return fmt.Errorf("failed to ftruncate memfd: %w", err)
	}

	// Mmap
	p.shmPtr, err = unix.Mmap(p.memfd, 0, p.shmSize, unix.PROT_READ|unix.PROT_WRITE, unix.MAP_SHARED)
	if err != nil {
		unix.Close(p.efdSend)
		unix.Close(p.efdAck)
		unix.Close(p.memfd)
		return fmt.Errorf("failed to mmap: %w", err)
	}

	// Wrap in os.File for ExtraFiles
	p.fileSend = os.NewFile(uintptr(p.efdSend), "efd_send")
	p.fileAck = os.NewFile(uintptr(p.efdAck), "efd_ack")
	p.fileShm = os.NewFile(uintptr(p.memfd), "efd_shm")

	// Prepare command
	p.cmd = exec.Command(p.childPath,
		"-mode", "child",
		"-fd-send", fmt.Sprintf("%d", p.fdSend),
		"-fd-ack", fmt.Sprintf("%d", p.fdAck),
		"-fd-shm", fmt.Sprintf("%d", p.fdShm),
		"-shm-size", fmt.Sprintf("%d", p.shmSize),
	)
	p.cmd.Stdout = os.Stdout
	p.cmd.Stderr = os.Stderr

	// Pass FDs. ExtraFiles starts at 3.
	// We pass 3 files: Send, Ack, Shm.
	// Child receives them at 3, 4, 5.
	// If child is Go, it needs to know this.
	p.cmd.ExtraFiles = []*os.File{p.fileSend, p.fileAck, p.fileShm}

	if err := p.cmd.Start(); err != nil {
		return fmt.Errorf("failed to start child: %w", err)
	}

	return nil
}

// SendData writes data to SHM and signals the child.
func (p *ShmParent) SendData(data []byte) error {
	if len(data) > p.shmSize {
		return fmt.Errorf("data too large for SHM")
	}

	// Write to SHM
	copy(p.shmPtr, data)

	// Send Length via EventFD
	buf := make([]byte, 8)
	binary.NativeEndian.PutUint64(buf, uint64(len(data)))

	if _, err := unix.Write(p.efdSend, buf); err != nil {
		return fmt.Errorf("write error: %w", err)
	}

	// Wait for ACK
	if _, err := unix.Read(p.efdAck, buf); err != nil {
		return fmt.Errorf("read error: %w", err)
	}

	return nil
}

// Close cleans up resources.
func (p *ShmParent) Close() {
	if p.cmd != nil && p.cmd.Process != nil {
		p.cmd.Process.Kill()
	}
	if p.shmPtr != nil {
		unix.Munmap(p.shmPtr)
	}
	if p.fileSend != nil {
		p.fileSend.Close()
	}
	if p.fileAck != nil {
		p.fileAck.Close()
	}
	if p.fileShm != nil {
		p.fileShm.Close()
	}
}

// ShmChild manages the receiver side.
type ShmChild struct {
	fdSend  int
	fdAck   int
	fdShm   int
	shmSize int
	shmPtr  []byte
}

// NewShmChild creates a new ShmChild instance.
func NewShmChild(fdSend, fdAck, fdShm, shmSize int) *ShmChild {
	return &ShmChild{
		fdSend:  fdSend,
		fdAck:   fdAck,
		fdShm:   fdShm,
		shmSize: shmSize,
	}
}

// Listen runs the loop.
func (c *ShmChild) Listen(handler func([]byte)) error {
	// Mmap SHM
	var err error
	c.shmPtr, err = unix.Mmap(c.fdShm, 0, c.shmSize, unix.PROT_READ, unix.MAP_SHARED)
	if err != nil {
		return fmt.Errorf("failed to mmap shm: %w", err)
	}
	defer unix.Munmap(c.shmPtr)

	buf := make([]byte, 8)
	native := binary.NativeEndian

	for {
		_, err := unix.Read(c.fdSend, buf)
		if err != nil {
			return fmt.Errorf("read error: %w", err)
		}
		length := native.Uint64(buf)
		if int(length) > c.shmSize {
			fmt.Printf("Received length %d exceeds SHM size %d\n", length, c.shmSize)
			continue
		}

		// Read from SHM
		data := c.shmPtr[:length]
		handler(data)

		// Send ACK (1)
		native.PutUint64(buf, 1)
		_, err = unix.Write(c.fdAck, buf)
		if err != nil {
			return fmt.Errorf("write error: %w", err)
		}
	}
}
