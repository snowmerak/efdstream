#include "efd.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>

void run_parent(const char *child_path, size_t shm_size) {
    shm_parent_t *parent = shm_parent_new(child_path, shm_size);
    if (!parent) {
        fprintf(stderr, "Failed to create parent\n");
        exit(1);
    }

    if (shm_parent_start(parent) != 0) {
        fprintf(stderr, "Failed to start parent\n");
        exit(1);
    }

    printf("[C Parent] Child started\n");

    for (int i = 0; i < 5; i++) {
        char msg[64];
        sprintf(msg, "Hello from C Parent %d", i);
        printf("[C Parent] Sending: %s\n", msg);
        
        if (shm_parent_send_data(parent, (uint8_t*)msg, strlen(msg)) != 0) {
            fprintf(stderr, "Communication error\n");
            break;
        }
        printf("[C Parent] Received ACK\n");

        size_t len;
        uint8_t *data = shm_parent_read_data(parent, &len);
        if (data) {
            char *recv_msg = strndup((char*)data, len);
            printf("[C Parent] Received: %s\n", recv_msg);
            free(recv_msg);
            free(data);
        } else {
            fprintf(stderr, "Read error\n");
        }

        sleep(1);
    }

    shm_parent_close(parent);
}

// Thread for sending data from child
void* child_sender_thread(void *arg) {
    shm_child_t *child = (shm_child_t*)arg;
    for (int i = 0; i < 5; i++) {
        usleep(500000); // 500ms
        char msg[64];
        sprintf(msg, "Hello from C Child %d", i);
        printf("[C Child] Sending: %s\n", msg);
        
        if (shm_child_send_data(child, (uint8_t*)msg, strlen(msg)) != 0) {
            fprintf(stderr, "[C Child] Send error\n");
            break;
        }
    }
    return NULL;
}

void child_handler(const uint8_t *data, size_t len) {
    char *msg = strndup((char*)data, len);
    printf("[C Child] Received: %s\n", msg);
    printf("[C Child] Sending ACK\n");
    free(msg);
}

void run_child(size_t shm_size) {
    printf("[C Child] Started with FDs: P2C(3,4,5) C2P(6,7,8)\n");

    shm_child_t *child = shm_child_new(shm_size);
    if (!child) {
        fprintf(stderr, "Failed to create child\n");
        exit(1);
    }

    // Create a separate sender instance for the thread because listen blocks
    // In a real app, you might want to protect shared resources or use non-blocking I/O.
    // Here, for simplicity, we create a second mapping or just share the struct if thread-safe.
    // Our struct is mostly thread-safe for distinct operations (send vs listen) except if they share buffers.
    // P2C and C2P are distinct, so it's okay to share 'child' pointer if we are careful.
    // Listen uses P2C read / P2C ack write.
    // Send uses C2P write / C2P ack read.
    // They are independent.

    pthread_t tid;
    pthread_create(&tid, NULL, child_sender_thread, child);

    shm_child_listen(child, child_handler);

    pthread_join(tid, NULL);
    shm_child_close(child);
}

int main(int argc, char *argv[]) {
    char *mode = "parent";
    char *child_path = "";
    size_t shm_size = 1024 * 1024;

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-mode") == 0 && i + 1 < argc) {
            mode = argv[++i];
        } else if (strcmp(argv[i], "-child") == 0 && i + 1 < argc) {
            child_path = argv[++i];
        } else if (strcmp(argv[i], "-shm-size") == 0 && i + 1 < argc) {
            shm_size = strtoul(argv[++i], NULL, 10);
        }
    }

    if (strcmp(mode, "parent") == 0) {
        if (strlen(child_path) == 0) {
            fprintf(stderr, "Child path is required in parent mode\n");
            return 1;
        }
        run_parent(child_path, shm_size);
    } else {
        run_child(shm_size);
    }

    return 0;
}
