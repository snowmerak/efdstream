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
	shmSize   int

	// Resources
	efdP2CSend int
	efdP2CAck  int
	memfdP2C   int
	shmP2CPtr  []byte

	efdC2PSend int
	efdC2PAck  int
	memfdC2P   int
	shmC2PPtr  []byte

	cmd *exec.Cmd

	fileP2CSend *os.File
	fileP2CAck  *os.File
	fileP2CShm  *os.File

	fileC2PSend *os.File
	fileC2PAck  *os.File
	fileC2PShm  *os.File
}

// NewShmParent creates a new ShmParent instance.
func NewShmParent(childPath string, shmSize int) *ShmParent {
	return &ShmParent{
		childPath: childPath,
		shmSize:   shmSize,
	}
}

// Start launches the child process and sets up resources.
func (p *ShmParent) Start() error {
	var err error
	// 1. Create P2C resources
	p.efdP2CSend, err = unix.Eventfd(0, 0)
	if err != nil {
		return fmt.Errorf("failed to create efdP2CSend: %w", err)
	}
	p.efdP2CAck, err = unix.Eventfd(0, 0)
	if err != nil {
		return fmt.Errorf("failed to create efdP2CAck: %w", err)
	}
	p.memfdP2C, err = unix.MemfdCreate("efdstream_shm_p2c", 0)
	if err != nil {
		return fmt.Errorf("failed to create memfdP2C: %w", err)
	}
	if err := unix.Ftruncate(p.memfdP2C, int64(p.shmSize)); err != nil {
		return fmt.Errorf("failed to ftruncate memfdP2C: %w", err)
	}
	p.shmP2CPtr, err = unix.Mmap(p.memfdP2C, 0, p.shmSize, unix.PROT_READ|unix.PROT_WRITE, unix.MAP_SHARED)
	if err != nil {
		return fmt.Errorf("failed to mmap P2C: %w", err)
	}

	// 2. Create C2P resources
	p.efdC2PSend, err = unix.Eventfd(0, 0)
	if err != nil {
		return fmt.Errorf("failed to create efdC2PSend: %w", err)
	}
	p.efdC2PAck, err = unix.Eventfd(0, 0)
	if err != nil {
		return fmt.Errorf("failed to create efdC2PAck: %w", err)
	}
	p.memfdC2P, err = unix.MemfdCreate("efdstream_shm_c2p", 0)
	if err != nil {
		return fmt.Errorf("failed to create memfdC2P: %w", err)
	}
	if err := unix.Ftruncate(p.memfdC2P, int64(p.shmSize)); err != nil {
		return fmt.Errorf("failed to ftruncate memfdC2P: %w", err)
	}
	p.shmC2PPtr, err = unix.Mmap(p.memfdC2P, 0, p.shmSize, unix.PROT_READ|unix.PROT_WRITE, unix.MAP_SHARED)
	if err != nil {
		return fmt.Errorf("failed to mmap C2P: %w", err)
	}

	// Wrap in os.File for ExtraFiles
	p.fileP2CSend = os.NewFile(uintptr(p.efdP2CSend), "efd_p2c_send")
	p.fileP2CAck = os.NewFile(uintptr(p.efdP2CAck), "efd_p2c_ack")
	p.fileP2CShm = os.NewFile(uintptr(p.memfdP2C), "efd_p2c_shm")

	p.fileC2PSend = os.NewFile(uintptr(p.efdC2PSend), "efd_c2p_send")
	p.fileC2PAck = os.NewFile(uintptr(p.efdC2PAck), "efd_c2p_ack")
	p.fileC2PShm = os.NewFile(uintptr(p.memfdC2P), "efd_c2p_shm")

	// Prepare command
	// We map the FDs to 3, 4, 5, 6, 7, 8 in the child process.
	// ExtraFiles[0] -> FD 3
	// ExtraFiles[1] -> FD 4
	// ...
	p.cmd = exec.Command(p.childPath,
		"-mode", "child",
		"-fd-p2c-send", "3",
		"-fd-p2c-ack", "4",
		"-fd-p2c-shm", "5",
		"-fd-c2p-send", "6",
		"-fd-c2p-ack", "7",
		"-fd-c2p-shm", "8",
		"-shm-size", fmt.Sprintf("%d", p.shmSize),
	)
	p.cmd.Stdout = os.Stdout
	p.cmd.Stderr = os.Stderr

	// Pass FDs. ExtraFiles starts at 3.
	// Order: P2C_Send, P2C_Ack, P2C_Shm, C2P_Send, C2P_Ack, C2P_Shm
	p.cmd.ExtraFiles = []*os.File{
		p.fileP2CSend, p.fileP2CAck, p.fileP2CShm,
		p.fileC2PSend, p.fileC2PAck, p.fileC2PShm,
	}

	if err := p.cmd.Start(); err != nil {
		return fmt.Errorf("failed to start child: %w", err)
	}

	return nil
}

// SendData sends data to the child (P2C).
func (p *ShmParent) SendData(data []byte) error {
	if len(data) > p.shmSize {
		return fmt.Errorf("data too large")
	}

	// Write to SHM
	copy(p.shmP2CPtr, data)

	// Signal
	lenBuf := make([]byte, 8)
	binary.LittleEndian.PutUint64(lenBuf, uint64(len(data)))
	if _, err := p.fileP2CSend.Write(lenBuf); err != nil {
		return err
	}

	// Wait for ACK
	ackBuf := make([]byte, 8)
	if _, err := p.fileP2CAck.Read(ackBuf); err != nil {
		return err
	}

	return nil
}

// ReadData reads data from the child (C2P).
func (p *ShmParent) ReadData() ([]byte, error) {
	// Wait for Signal
	lenBuf := make([]byte, 8)
	if _, err := p.fileC2PSend.Read(lenBuf); err != nil {
		return nil, err
	}
	length := binary.LittleEndian.Uint64(lenBuf)

	if int(length) > p.shmSize {
		return nil, fmt.Errorf("received length %d exceeds SHM size", length)
	}

	// Read from SHM
	data := make([]byte, length)
	copy(data, p.shmC2PPtr[:length])

	// Send ACK
	ackBuf := make([]byte, 8)
	binary.LittleEndian.PutUint64(ackBuf, 1)
	if _, err := p.fileC2PAck.Write(ackBuf); err != nil {
		return nil, err
	}

	return data, nil
}

// Close cleans up resources.
func (p *ShmParent) Close() {
	if p.shmP2CPtr != nil {
		unix.Munmap(p.shmP2CPtr)
	}
	if p.shmC2PPtr != nil {
		unix.Munmap(p.shmC2PPtr)
	}
	if p.cmd != nil && p.cmd.Process != nil {
		p.cmd.Process.Kill()
	}
}

// ShmChild manages the child side of the connection.
type ShmChild struct {
	fdP2CSend int
	fdP2CAck  int
	fdP2CShm  int
	fdC2PSend int
	fdC2PAck  int
	fdC2PShm  int
	shmSize   int

	shmP2CPtr []byte
	shmC2PPtr []byte

	fileP2CSend *os.File
	fileP2CAck  *os.File
	fileC2PSend *os.File
	fileC2PAck  *os.File
}

// NewShmChild creates a new ShmChild instance.
func NewShmChild(fdP2CSend, fdP2CAck, fdP2CShm, fdC2PSend, fdC2PAck, fdC2PShm, shmSize int) (*ShmChild, error) {
	c := &ShmChild{
		fdP2CSend: fdP2CSend,
		fdP2CAck:  fdP2CAck,
		fdP2CShm:  fdP2CShm,
		fdC2PSend: fdC2PSend,
		fdC2PAck:  fdC2PAck,
		fdC2PShm:  fdC2PShm,
		shmSize:   shmSize,
	}

	var err error
	// Mmap P2C (Read)
	c.shmP2CPtr, err = unix.Mmap(c.fdP2CShm, 0, c.shmSize, unix.PROT_READ, unix.MAP_SHARED)
	if err != nil {
		return nil, fmt.Errorf("failed to mmap P2C: %w", err)
	}

	// Mmap C2P (Write)
	c.shmC2PPtr, err = unix.Mmap(c.fdC2PShm, 0, c.shmSize, unix.PROT_READ|unix.PROT_WRITE, unix.MAP_SHARED)
	if err != nil {
		return nil, fmt.Errorf("failed to mmap C2P: %w", err)
	}

	c.fileP2CSend = os.NewFile(uintptr(c.fdP2CSend), "efd_p2c_send")
	c.fileP2CAck = os.NewFile(uintptr(c.fdP2CAck), "efd_p2c_ack")
	c.fileC2PSend = os.NewFile(uintptr(c.fdC2PSend), "efd_c2p_send")
	c.fileC2PAck = os.NewFile(uintptr(c.fdC2PAck), "efd_c2p_ack")

	return c, nil
}

// Listen reads data from the parent (P2C).
func (c *ShmChild) Listen(handler func([]byte)) error {
	lenBuf := make([]byte, 8)
	ackBuf := make([]byte, 8)
	binary.LittleEndian.PutUint64(ackBuf, 1)

	for {
		if _, err := c.fileP2CSend.Read(lenBuf); err != nil {
			return err
		}
		length := binary.LittleEndian.Uint64(lenBuf)

		if int(length) > c.shmSize {
			fmt.Printf("Received length %d exceeds SHM size\n", length)
			continue
		}

		data := make([]byte, length)
		copy(data, c.shmP2CPtr[:length])
		handler(data)

		if _, err := c.fileP2CAck.Write(ackBuf); err != nil {
			return err
		}
	}
}

// SendData sends data to the parent (C2P).
func (c *ShmChild) SendData(data []byte) error {
	if len(data) > c.shmSize {
		return fmt.Errorf("data too large")
	}

	// Write to SHM
	copy(c.shmC2PPtr, data)

	// Signal
	lenBuf := make([]byte, 8)
	binary.LittleEndian.PutUint64(lenBuf, uint64(len(data)))
	if _, err := c.fileC2PSend.Write(lenBuf); err != nil {
		return err
	}

	// Wait for ACK
	ackBuf := make([]byte, 8)
	if _, err := c.fileC2PAck.Read(ackBuf); err != nil {
		return err
	}

	return nil
}

// Close cleans up resources.
func (c *ShmChild) Close() {
	if c.shmP2CPtr != nil {
		unix.Munmap(c.shmP2CPtr)
	}
	if c.shmC2PPtr != nil {
		unix.Munmap(c.shmC2PPtr)
	}
}
