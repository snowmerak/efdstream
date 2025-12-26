#define _GNU_SOURCE
#include "efd.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/eventfd.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <errno.h>

// --- Parent Implementation ---

shm_parent_t* shm_parent_new(const char *child_path, size_t shm_size) {
    shm_parent_t *p = (shm_parent_t*)malloc(sizeof(shm_parent_t));
    if (!p) return NULL;
    memset(p, 0, sizeof(shm_parent_t));
    p->child_path = strdup(child_path);
    p->shm_size = shm_size;
    return p;
}

int shm_parent_start(shm_parent_t *p) {
    // 1. Create P2C resources
    p->efd_p2c_send = eventfd(0, 0);
    if (p->efd_p2c_send == -1) return -1;
    p->efd_p2c_ack = eventfd(0, 0);
    if (p->efd_p2c_ack == -1) return -1;
    p->memfd_p2c = memfd_create("efdstream_shm_p2c", 0);
    if (p->memfd_p2c == -1) return -1;
    if (ftruncate(p->memfd_p2c, p->shm_size) == -1) return -1;
    p->shm_p2c_ptr = mmap(NULL, p->shm_size, PROT_READ | PROT_WRITE, MAP_SHARED, p->memfd_p2c, 0);
    if (p->shm_p2c_ptr == MAP_FAILED) return -1;

    // 2. Create C2P resources
    p->efd_c2p_send = eventfd(0, 0);
    if (p->efd_c2p_send == -1) return -1;
    p->efd_c2p_ack = eventfd(0, 0);
    if (p->efd_c2p_ack == -1) return -1;
    p->memfd_c2p = memfd_create("efdstream_shm_c2p", 0);
    if (p->memfd_c2p == -1) return -1;
    if (ftruncate(p->memfd_c2p, p->shm_size) == -1) return -1;
    p->shm_c2p_ptr = mmap(NULL, p->shm_size, PROT_READ | PROT_WRITE, MAP_SHARED, p->memfd_c2p, 0);
    if (p->shm_c2p_ptr == MAP_FAILED) return -1;

    // 3. Fork and Exec
    pid_t pid = fork();
    if (pid == -1) {
        return -1;
    } else if (pid == 0) {
        // Child process
        // Map FDs to 3, 4, 5, 6, 7, 8
        if (dup2(p->efd_p2c_send, 3) == -1) exit(1);
        if (dup2(p->efd_p2c_ack, 4) == -1) exit(1);
        if (dup2(p->memfd_p2c, 5) == -1) exit(1);
        if (dup2(p->efd_c2p_send, 6) == -1) exit(1);
        if (dup2(p->efd_c2p_ack, 7) == -1) exit(1);
        if (dup2(p->memfd_c2p, 8) == -1) exit(1);

        // Close originals if they are not the target FDs (to be safe, though dup2 handles overlap)
        // In a real robust implementation, we should close all other FDs.
        // For now, we rely on the fact that we just dup2'd what we need.

        char shm_size_str[32];
        sprintf(shm_size_str, "%zu", p->shm_size);

        // Prepare args
        // We pass the flags just for compatibility, even though we use fixed FDs.
        char *args[] = {
            p->child_path,
            "-mode", "child",
            "-fd-p2c-send", "3",
            "-fd-p2c-ack", "4",
            "-fd-p2c-shm", "5",
            "-fd-c2p-send", "6",
            "-fd-c2p-ack", "7",
            "-fd-c2p-shm", "8",
            "-shm-size", shm_size_str,
            NULL
        };

        execv(p->child_path, args);
        perror("execv failed");
        exit(1);
    } else {
        // Parent process
        p->child_pid = pid;
    }

    return 0;
}

int shm_parent_send_data(shm_parent_t *p, const uint8_t *data, size_t len) {
    if (len > p->shm_size) return -1;

    // Write to SHM
    memcpy(p->shm_p2c_ptr, data, len);

    // Signal Length
    uint64_t len_val = (uint64_t)len;
    if (write(p->efd_p2c_send, &len_val, 8) != 8) return -1;

    // Wait for ACK
    uint64_t ack_val;
    if (read(p->efd_p2c_ack, &ack_val, 8) != 8) return -1;

    return 0;
}

uint8_t* shm_parent_read_data(shm_parent_t *p, size_t *len) {
    // Wait for Signal
    uint64_t len_val;
    if (read(p->efd_c2p_send, &len_val, 8) != 8) return NULL;

    if (len_val > p->shm_size) return NULL;

    *len = (size_t)len_val;
    uint8_t *data = (uint8_t*)malloc(*len);
    if (!data) return NULL;

    // Read from SHM
    memcpy(data, p->shm_c2p_ptr, *len);

    // Send ACK
    uint64_t ack_val = 1;
    if (write(p->efd_c2p_ack, &ack_val, 8) != 8) {
        free(data);
        return NULL;
    }

    return data;
}

void shm_parent_close(shm_parent_t *p) {
    if (p->shm_p2c_ptr && p->shm_p2c_ptr != MAP_FAILED) munmap(p->shm_p2c_ptr, p->shm_size);
    if (p->shm_c2p_ptr && p->shm_c2p_ptr != MAP_FAILED) munmap(p->shm_c2p_ptr, p->shm_size);
    
    if (p->efd_p2c_send != -1) close(p->efd_p2c_send);
    if (p->efd_p2c_ack != -1) close(p->efd_p2c_ack);
    if (p->memfd_p2c != -1) close(p->memfd_p2c);
    
    if (p->efd_c2p_send != -1) close(p->efd_c2p_send);
    if (p->efd_c2p_ack != -1) close(p->efd_c2p_ack);
    if (p->memfd_c2p != -1) close(p->memfd_c2p);

    if (p->child_pid > 0) {
        kill(p->child_pid, SIGTERM);
        waitpid(p->child_pid, NULL, 0);
    }
    
    free(p->child_path);
    free(p);
}

// --- Child Implementation ---

shm_child_t* shm_child_new(size_t shm_size) {
    shm_child_t *c = (shm_child_t*)malloc(sizeof(shm_child_t));
    if (!c) return NULL;
    memset(c, 0, sizeof(shm_child_t));
    c->shm_size = shm_size;

    // Fixed FDs
    c->fd_p2c_send = 3;
    c->fd_p2c_ack = 4;
    c->fd_p2c_shm = 5;
    c->fd_c2p_send = 6;
    c->fd_c2p_ack = 7;
    c->fd_c2p_shm = 8;

    // Mmap P2C (Read)
    c->shm_p2c_ptr = mmap(NULL, c->shm_size, PROT_READ, MAP_SHARED, c->fd_p2c_shm, 0);
    if (c->shm_p2c_ptr == MAP_FAILED) {
        free(c);
        return NULL;
    }

    // Mmap C2P (Write)
    c->shm_c2p_ptr = mmap(NULL, c->shm_size, PROT_READ | PROT_WRITE, MAP_SHARED, c->fd_c2p_shm, 0);
    if (c->shm_c2p_ptr == MAP_FAILED) {
        munmap(c->shm_p2c_ptr, c->shm_size);
        free(c);
        return NULL;
    }

    return c;
}

int shm_child_listen(shm_child_t *c, child_listen_cb handler) {
    uint64_t len_val;
    uint64_t ack_val = 1;

    while (1) {
        if (read(c->fd_p2c_send, &len_val, 8) != 8) return -1;
        
        if (len_val > c->shm_size) {
            fprintf(stderr, "Received length %lu exceeds SHM size\n", len_val);
            continue;
        }

        handler(c->shm_p2c_ptr, (size_t)len_val);

        if (write(c->fd_p2c_ack, &ack_val, 8) != 8) return -1;
    }
    return 0;
}

int shm_child_send_data(shm_child_t *c, const uint8_t *data, size_t len) {
    if (len > c->shm_size) return -1;

    // Write to SHM
    memcpy(c->shm_c2p_ptr, data, len);

    // Signal
    uint64_t len_val = (uint64_t)len;
    if (write(c->fd_c2p_send, &len_val, 8) != 8) return -1;

    // Wait for ACK
    uint64_t ack_val;
    if (read(c->fd_c2p_ack, &ack_val, 8) != 8) return -1;

    return 0;
}

void shm_child_close(shm_child_t *c) {
    if (c->shm_p2c_ptr && c->shm_p2c_ptr != MAP_FAILED) munmap(c->shm_p2c_ptr, c->shm_size);
    if (c->shm_c2p_ptr && c->shm_c2p_ptr != MAP_FAILED) munmap(c->shm_c2p_ptr, c->shm_size);
    free(c);
}
