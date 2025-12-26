#ifndef EFD_H
#define EFD_H

#include <stddef.h>
#include <stdint.h>

// Parent Structure
typedef struct {
    char *child_path;
    size_t shm_size;

    // Resources
    int efd_p2c_send;
    int efd_p2c_ack;
    int memfd_p2c;
    uint8_t *shm_p2c_ptr;

    int efd_c2p_send;
    int efd_c2p_ack;
    int memfd_c2p;
    uint8_t *shm_c2p_ptr;

    int child_pid;
} shm_parent_t;

// Child Structure
typedef struct {
    size_t shm_size;

    // Fixed FDs
    int fd_p2c_send;
    int fd_p2c_ack;
    int fd_p2c_shm;

    int fd_c2p_send;
    int fd_c2p_ack;
    int fd_c2p_shm;

    uint8_t *shm_p2c_ptr;
    uint8_t *shm_c2p_ptr;
} shm_child_t;

// Parent Functions
shm_parent_t* shm_parent_new(const char *child_path, size_t shm_size);
int shm_parent_start(shm_parent_t *parent);
int shm_parent_send_data(shm_parent_t *parent, const uint8_t *data, size_t len);
// Returns allocated buffer, caller must free. len is output.
uint8_t* shm_parent_read_data(shm_parent_t *parent, size_t *len);
void shm_parent_close(shm_parent_t *parent);

// Child Functions
shm_child_t* shm_child_new(size_t shm_size);
// Callback type for listen
typedef void (*child_listen_cb)(const uint8_t *data, size_t len);
int shm_child_listen(shm_child_t *child, child_listen_cb handler);
int shm_child_send_data(shm_child_t *child, const uint8_t *data, size_t len);
void shm_child_close(shm_child_t *child);

#endif // EFD_H
