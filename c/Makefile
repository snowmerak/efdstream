CC = gcc
CFLAGS = -Wall -O2 -pthread
TARGET = efdstream_c

all: $(TARGET)

$(TARGET): main.c efd.c
	$(CC) $(CFLAGS) -o $(TARGET) main.c efd.c

clean:
	rm -f $(TARGET)
