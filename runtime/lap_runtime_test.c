#define _POSIX_C_SOURCE 200809L

#include <assert.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

extern void lap_extern(uint64_t operation, uint64_t argument0,
                       uint64_t argument1, uint64_t argument2,
                       uint64_t argument3, uint64_t *out);
extern bool lap_runtime_start(int argument_count, char **argument_values);
extern void lap_runtime_finish(void);

enum {
  OPERATION_PROCESS_EXIT = 0x0000,
  OPERATION_MEMORY_ACQUIRE = 0x0100,
  OPERATION_MEMORY_RELEASE = 0x0101,
  OPERATION_MEMORY_READ = 0x0102,
  OPERATION_MEMORY_WRITE = 0x0103,
  OPERATION_STREAM_OPEN = 0x0200,
  OPERATION_STREAM_READ = 0x0201,
  OPERATION_STREAM_WRITE = 0x0202,
  OPERATION_STREAM_FLUSH = 0x0203,
  OPERATION_STREAM_CLOSE = 0x0204,
  OPERATION_CLOCK_MONOTONIC = 0x0300,
  OPERATION_CLOCK_REALTIME = 0x0301,
  OPERATION_RANDOM_BYTE = 0x0400,
};

enum {
  STREAM_STANDARD_INPUT = 0,
  STREAM_STANDARD_OUTPUT = 1,
  STREAM_STANDARD_ERROR = 2,
  STREAM_ARGUMENTS = 3,
};

enum {
  STREAM_MODE_READ = 0,
  STREAM_MODE_WRITE = 1,
  STREAM_MODE_APPEND = 2,
};

enum {
  TABLE_GROWTH_BLOCKS = 64,
  TABLE_GROWTH_STREAMS = 32,
  BUFFER_PROBE_LIMIT = 1 << 20,
  REFILL_FILE_SIZE = 1 << 16,
  RANDOM_SAMPLE_SIZE = 256,
  PATH_CAPACITY = 256,
};

static char *program_path;
static unsigned char probe_bytes[BUFFER_PROBE_LIMIT];

static bool invoke(uint64_t operation, uint64_t argument0, uint64_t argument1,
                   uint64_t argument2, uint64_t argument3, uint64_t *payload) {
  uint64_t result[2];
  lap_extern(operation, argument0, argument1, argument2, argument3, result);
  if (result[0] == 0) {
    assert(result[1] == 0);
    return false;
  }
  *payload = result[1];
  return true;
}

static uint64_t succeeds(uint64_t operation, uint64_t argument0,
                         uint64_t argument1, uint64_t argument2,
                         uint64_t argument3) {
  uint64_t payload = 0;
  assert(
      invoke(operation, argument0, argument1, argument2, argument3, &payload));
  return payload;
}

static void fails(uint64_t operation, uint64_t argument0, uint64_t argument1,
                  uint64_t argument2, uint64_t argument3) {
  uint64_t payload = 0;
  assert(
      !invoke(operation, argument0, argument1, argument2, argument3, &payload));
}

static uint64_t acquire_block(uint64_t length) {
  return succeeds(OPERATION_MEMORY_ACQUIRE, length, 0, 0, 0);
}

static void release_block(uint64_t handle) {
  succeeds(OPERATION_MEMORY_RELEASE, handle, 0, 0, 0);
}

static unsigned char read_block(uint64_t handle, uint64_t offset) {
  return (unsigned char)succeeds(OPERATION_MEMORY_READ, handle, offset, 0, 0);
}

static void write_block(uint64_t handle, uint64_t offset, unsigned char byte) {
  succeeds(OPERATION_MEMORY_WRITE, handle, offset, byte, 0);
}

static uint64_t open_stream(uint64_t path_handle, uint64_t path_offset,
                            uint64_t path_length, uint64_t mode) {
  return succeeds(OPERATION_STREAM_OPEN, path_handle, path_offset, path_length,
                  mode);
}

static unsigned char read_stream(uint64_t stream) {
  return (unsigned char)succeeds(OPERATION_STREAM_READ, stream, 0, 0, 0);
}

static void write_stream(uint64_t stream, unsigned char byte) {
  succeeds(OPERATION_STREAM_WRITE, stream, byte, 0, 0);
}

static void flush_stream(uint64_t stream) {
  succeeds(OPERATION_STREAM_FLUSH, stream, 0, 0, 0);
}

static void close_stream(uint64_t stream) {
  succeeds(OPERATION_STREAM_CLOSE, stream, 0, 0, 0);
}

static uint64_t path_block(const char *path) {
  uint64_t length = (uint64_t)strlen(path);
  uint64_t handle = acquire_block(length);
  for (uint64_t index = 0; index < length; index++) {
    write_block(handle, index, (unsigned char)path[index]);
  }
  return handle;
}

static uint64_t open_named_stream(const char *path, uint64_t mode) {
  uint64_t block = path_block(path);
  uint64_t stream = open_stream(block, 0, (uint64_t)strlen(path), mode);
  release_block(block);
  return stream;
}

static void temporary_path(char *path, size_t capacity, const char *name) {
  int count = snprintf(path, capacity, "/tmp/lap_runtime_test_%d_%s",
                       (int)getpid(), name);
  assert(count > 0);
  assert((size_t)count < capacity);
  unlink(path);
}

static off_t file_size(const char *path) {
  struct stat status;
  assert(stat(path, &status) == 0);
  return status.st_size;
}

static size_t read_file(const char *path, unsigned char *bytes,
                        size_t capacity) {
  int descriptor = open(path, O_RDONLY);
  assert(descriptor >= 0);
  ssize_t count = read(descriptor, bytes, capacity);
  assert(count >= 0);
  close(descriptor);
  return (size_t)count;
}

static void write_file(const char *path, const unsigned char *bytes,
                       size_t length) {
  int descriptor = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0666);
  assert(descriptor >= 0);
  ssize_t written = write(descriptor, bytes, length);
  assert(written == (ssize_t)length);
  close(descriptor);
}

static void test_memory_operations(void) {
  uint64_t first = acquire_block(4);
  uint64_t second = acquire_block(4);
  assert(first != second);

  write_block(first, 0, 0x12);
  write_block(first, 3, 0x34);
  assert(read_block(first, 0) == 0x12);
  assert(read_block(first, 3) == 0x34);
  write_block(second, 0, 0x00);
  write_block(first, 0, 0xFF);
  assert(read_block(first, 0) == 0xFF);
  assert(read_block(second, 0) == 0x00);

  fails(OPERATION_MEMORY_READ, first, 4, 0, 0);
  fails(OPERATION_MEMORY_WRITE, first, 4, 0x01, 0);
  fails(OPERATION_MEMORY_READ, first, UINT64_MAX, 0, 0);
  fails(OPERATION_MEMORY_WRITE, first, UINT64_MAX, 0x01, 0);
  fails(OPERATION_MEMORY_READ, UINT64_MAX, 0, 0, 0);
  fails(OPERATION_MEMORY_WRITE, UINT64_MAX, 0, 0x01, 0);
  fails(OPERATION_MEMORY_RELEASE, UINT64_MAX, 0, 0, 0);

  release_block(first);
  fails(OPERATION_MEMORY_READ, first, 0, 0, 0);
  fails(OPERATION_MEMORY_WRITE, first, 0, 0x01, 0);
  fails(OPERATION_MEMORY_RELEASE, first, 0, 0, 0);
  release_block(second);

  uint64_t empty = acquire_block(0);
  fails(OPERATION_MEMORY_READ, empty, 0, 0, 0);
  fails(OPERATION_MEMORY_WRITE, empty, 0, 0x01, 0);
  release_block(empty);

  fails(OPERATION_MEMORY_ACQUIRE, UINT64_MAX, 0, 0, 0);
  uint64_t recovered = acquire_block(1);
  write_block(recovered, 0, 0xAB);
  assert(read_block(recovered, 0) == 0xAB);
  release_block(recovered);

  uint64_t masked = acquire_block(1);
  succeeds(OPERATION_MEMORY_WRITE, masked, 0, 0x1FF, 0);
  assert(read_block(masked, 0) == 0xFF);
  release_block(masked);

  uint64_t blocks[TABLE_GROWTH_BLOCKS];
  for (uint64_t index = 0; index < TABLE_GROWTH_BLOCKS; index++) {
    blocks[index] = acquire_block(1);
    write_block(blocks[index], 0, (unsigned char)index);
  }
  for (uint64_t index = 0; index < TABLE_GROWTH_BLOCKS; index++) {
    assert(read_block(blocks[index], 0) == (unsigned char)index);
    release_block(blocks[index]);
  }
}

static void test_process_exit_flushes(void) {
  char path[PATH_CAPACITY];
  temporary_path(path, sizeof path, "exit");

  pid_t child = fork();
  assert(child >= 0);
  if (child == 0) {
    int descriptor = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0666);
    assert(descriptor >= 0);
    assert(dup2(descriptor, STREAM_STANDARD_OUTPUT) == STREAM_STANDARD_OUTPUT);
    close(descriptor);
    write_stream(STREAM_STANDARD_OUTPUT, 'x');
    uint64_t ignored[2] = {0, 0};
    lap_extern(OPERATION_PROCESS_EXIT, 7, 0, 0, 0, ignored);
    _exit(99);
  }

  int status = 0;
  assert(waitpid(child, &status, 0) == child);
  assert(WIFEXITED(status));
  assert(WEXITSTATUS(status) == 7);

  unsigned char bytes[4];
  assert(read_file(path, bytes, sizeof bytes) == 1);
  assert(bytes[0] == 'x');
  unlink(path);
}

static void test_standard_input(void) {
  char path[PATH_CAPACITY];
  temporary_path(path, sizeof path, "input");
  const unsigned char content[] = {'x', 'y', 'z'};
  write_file(path, content, sizeof content);

  int saved = dup(STREAM_STANDARD_INPUT);
  assert(saved >= 0);
  int descriptor = open(path, O_RDONLY);
  assert(descriptor >= 0);
  assert(dup2(descriptor, STREAM_STANDARD_INPUT) == STREAM_STANDARD_INPUT);
  close(descriptor);

  assert(read_stream(STREAM_STANDARD_INPUT) == 'x');
  assert(read_stream(STREAM_STANDARD_INPUT) == 'y');
  assert(read_stream(STREAM_STANDARD_INPUT) == 'z');
  fails(OPERATION_STREAM_READ, STREAM_STANDARD_INPUT, 0, 0, 0);
  flush_stream(STREAM_STANDARD_INPUT);
  fails(OPERATION_STREAM_WRITE, STREAM_STANDARD_INPUT, 'q', 0, 0);

  assert(dup2(saved, STREAM_STANDARD_INPUT) == STREAM_STANDARD_INPUT);
  close(saved);
  unlink(path);
}

static void test_standard_output_buffering(void) {
  char path[PATH_CAPACITY];
  temporary_path(path, sizeof path, "buffered");

  int saved = dup(STREAM_STANDARD_OUTPUT);
  assert(saved >= 0);
  int descriptor = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0666);
  assert(descriptor >= 0);
  assert(dup2(descriptor, STREAM_STANDARD_OUTPUT) == STREAM_STANDARD_OUTPUT);
  close(descriptor);

  uint64_t written = 0;
  while (file_size(path) == 0) {
    assert(written < BUFFER_PROBE_LIMIT);
    write_stream(STREAM_STANDARD_OUTPUT, (unsigned char)(written * 37 + 11));
    written++;
  }

  flush_stream(STREAM_STANDARD_OUTPUT);
  size_t count = read_file(path, probe_bytes, sizeof probe_bytes);
  assert(count == written);
  for (size_t index = 0; index < count; index++) {
    assert(probe_bytes[index] == (unsigned char)((uint64_t)index * 37 + 11));
  }

  assert(dup2(saved, STREAM_STANDARD_OUTPUT) == STREAM_STANDARD_OUTPUT);
  close(saved);
  unlink(path);
}

static void test_standard_error_unbuffered(void) {
  char path[PATH_CAPACITY];
  temporary_path(path, sizeof path, "unbuffered");

  int saved = dup(STREAM_STANDARD_ERROR);
  assert(saved >= 0);
  int descriptor = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0666);
  assert(descriptor >= 0);
  assert(dup2(descriptor, STREAM_STANDARD_ERROR) == STREAM_STANDARD_ERROR);
  close(descriptor);

  write_stream(STREAM_STANDARD_ERROR, 'E');
  assert(file_size(path) == 1);
  unsigned char bytes[4];
  assert(read_file(path, bytes, sizeof bytes) == 1);
  assert(bytes[0] == 'E');
  flush_stream(STREAM_STANDARD_ERROR);
  fails(OPERATION_STREAM_READ, STREAM_STANDARD_ERROR, 0, 0, 0);

  assert(dup2(saved, STREAM_STANDARD_ERROR) == STREAM_STANDARD_ERROR);
  close(saved);
  unlink(path);
}

static void test_file_streams(void) {
  char path[PATH_CAPACITY];
  temporary_path(path, sizeof path, "file");
  uint64_t path_handle = path_block(path);
  uint64_t path_length = (uint64_t)strlen(path);

  fails(OPERATION_STREAM_OPEN, path_handle, 0, path_length, 3);
  fails(OPERATION_STREAM_OPEN, path_handle, 0, path_length, 0xFF);
  fails(OPERATION_STREAM_OPEN, path_handle, path_length, 0, STREAM_MODE_READ);
  fails(OPERATION_STREAM_OPEN, path_handle, path_length + 1, 0,
        STREAM_MODE_READ);
  fails(OPERATION_STREAM_OPEN, path_handle, 0, UINT64_MAX, STREAM_MODE_READ);
  fails(OPERATION_STREAM_OPEN, path_handle, UINT64_MAX, 0, STREAM_MODE_READ);
  fails(OPERATION_STREAM_OPEN, path_handle, path_length - 1, UINT64_MAX,
        STREAM_MODE_READ);
  fails(OPERATION_STREAM_OPEN, UINT64_MAX, 0, 4, STREAM_MODE_READ);
  fails(OPERATION_STREAM_READ, UINT64_MAX, 0, 0, 0);
  fails(OPERATION_STREAM_WRITE, UINT64_MAX, 'x', 0, 0);
  fails(OPERATION_STREAM_FLUSH, UINT64_MAX, 0, 0, 0);
  fails(OPERATION_STREAM_CLOSE, UINT64_MAX, 0, 0, 0);

  uint64_t writer = open_stream(path_handle, 0, path_length, STREAM_MODE_WRITE);
  assert(file_size(path) == 0);
  const unsigned char hello[] = "hello";
  for (size_t index = 0; index < sizeof hello - 1; index++) {
    write_stream(writer, hello[index]);
  }
  succeeds(OPERATION_STREAM_WRITE, writer, 0x1FF, 0, 0);
  assert(file_size(path) == 0);
  flush_stream(writer);
  assert(file_size(path) == 6);
  unsigned char bytes[32];
  assert(read_file(path, bytes, sizeof bytes) == 6);
  assert(memcmp(bytes, "hello", 5) == 0);
  assert(bytes[5] == 0xFF);
  close_stream(writer);

  uint64_t appender =
      open_stream(path_handle, 0, path_length, STREAM_MODE_APPEND);
  assert(file_size(path) == 6);
  write_stream(appender, '!');
  close_stream(appender);
  assert(file_size(path) == 7);
  assert(read_file(path, bytes, sizeof bytes) == 7);
  assert(memcmp(bytes, "hello\xFF!", 7) == 0);

  uint64_t truncator =
      open_stream(path_handle, 0, path_length, STREAM_MODE_WRITE);
  assert(file_size(path) == 0);
  write_stream(truncator, 'x');
  close_stream(truncator);
  assert(file_size(path) == 1);
  assert(read_file(path, bytes, sizeof bytes) == 1);
  assert(bytes[0] == 'x');

  uint64_t reader = open_stream(path_handle, 0, path_length, STREAM_MODE_READ);
  assert(read_stream(reader) == 'x');
  fails(OPERATION_STREAM_READ, reader, 0, 0, 0);
  flush_stream(reader);
  fails(OPERATION_STREAM_WRITE, reader, 'y', 0, 0);
  close_stream(reader);
  fails(OPERATION_STREAM_READ, reader, 0, 0, 0);
  fails(OPERATION_STREAM_WRITE, reader, 'y', 0, 0);
  fails(OPERATION_STREAM_FLUSH, reader, 0, 0, 0);
  fails(OPERATION_STREAM_CLOSE, reader, 0, 0, 0);

  release_block(path_handle);
  fails(OPERATION_STREAM_OPEN, path_handle, 0, path_length, STREAM_MODE_READ);

  char absent[PATH_CAPACITY];
  temporary_path(absent, sizeof absent, "absent");
  uint64_t absent_handle = path_block(absent);
  fails(OPERATION_STREAM_OPEN, absent_handle, 0, (uint64_t)strlen(absent),
        STREAM_MODE_READ);
  release_block(absent_handle);

  unlink(path);
}

static void test_flush_failure_discards_buffer(void) {
  uint64_t stream = open_named_stream("/dev/full", STREAM_MODE_WRITE);
  write_stream(stream, 'x');
  fails(OPERATION_STREAM_FLUSH, stream, 0, 0, 0);
  flush_stream(stream);

  uint64_t payload = 0;
  uint64_t attempts = 0;
  while (invoke(OPERATION_STREAM_WRITE, stream, 'y', 0, 0, &payload)) {
    attempts++;
    assert(attempts < BUFFER_PROBE_LIMIT);
  }
  flush_stream(stream);

  write_stream(stream, 'z');
  fails(OPERATION_STREAM_CLOSE, stream, 0, 0, 0);
  fails(OPERATION_STREAM_WRITE, stream, 'w', 0, 0);
  fails(OPERATION_STREAM_FLUSH, stream, 0, 0, 0);
  fails(OPERATION_STREAM_CLOSE, stream, 0, 0, 0);
}

static void test_stream_path_window(void) {
  char path[PATH_CAPACITY];
  temporary_path(path, sizeof path, "window");
  size_t length = strlen(path);
  uint64_t block = acquire_block((uint64_t)length + 2);
  write_block(block, 0, 'x');
  for (size_t index = 0; index < length; index++) {
    write_block(block, index + 1, (unsigned char)path[index]);
  }
  write_block(block, length + 1, 'y');
  uint64_t writer = open_stream(block, 1, (uint64_t)length, STREAM_MODE_WRITE);
  write_stream(writer, 'w');
  close_stream(writer);
  assert(file_size(path) == 1);
  unsigned char bytes[4];
  assert(read_file(path, bytes, sizeof bytes) == 1);
  assert(bytes[0] == 'w');
  release_block(block);
  unlink(path);
}

static void test_independent_stream_buffers(void) {
  char first_path[PATH_CAPACITY];
  char second_path[PATH_CAPACITY];
  temporary_path(first_path, sizeof first_path, "first");
  temporary_path(second_path, sizeof second_path, "second");
  uint64_t first = open_named_stream(first_path, STREAM_MODE_WRITE);
  uint64_t second = open_named_stream(second_path, STREAM_MODE_WRITE);

  write_stream(first, 'a');
  write_stream(second, 'b');
  assert(file_size(first_path) == 0);
  assert(file_size(second_path) == 0);

  flush_stream(first);
  assert(file_size(first_path) == 1);
  assert(file_size(second_path) == 0);
  unsigned char bytes[4];
  assert(read_file(first_path, bytes, sizeof bytes) == 1);
  assert(bytes[0] == 'a');

  flush_stream(second);
  assert(read_file(second_path, bytes, sizeof bytes) == 1);
  assert(bytes[0] == 'b');

  close_stream(first);
  close_stream(second);
  unlink(first_path);
  unlink(second_path);
}

static void test_stream_table_growth(void) {
  char path[PATH_CAPACITY];
  temporary_path(path, sizeof path, "growth");
  uint64_t streams[TABLE_GROWTH_STREAMS];
  for (uint64_t index = 0; index < TABLE_GROWTH_STREAMS; index++) {
    streams[index] = open_named_stream(path, STREAM_MODE_APPEND);
  }
  for (uint64_t index = 0; index < TABLE_GROWTH_STREAMS; index++) {
    write_stream(streams[index], (unsigned char)index);
  }
  for (uint64_t index = 0; index < TABLE_GROWTH_STREAMS; index++) {
    flush_stream(streams[index]);
  }
  for (uint64_t index = 0; index < TABLE_GROWTH_STREAMS; index++) {
    close_stream(streams[index]);
  }
  unsigned char bytes[TABLE_GROWTH_STREAMS];
  assert(read_file(path, bytes, sizeof bytes) == TABLE_GROWTH_STREAMS);
  for (uint64_t index = 0; index < TABLE_GROWTH_STREAMS; index++) {
    assert(bytes[index] == (unsigned char)index);
  }
  unlink(path);
}

static void test_stream_refill_reading(void) {
  char path[PATH_CAPACITY];
  temporary_path(path, sizeof path, "refill");
  static unsigned char content[REFILL_FILE_SIZE];
  for (size_t index = 0; index < sizeof content; index++) {
    content[index] = (unsigned char)(index * 101 + 7);
  }
  write_file(path, content, sizeof content);

  uint64_t reader = open_named_stream(path, STREAM_MODE_READ);
  for (size_t index = 0; index < sizeof content; index++) {
    assert(read_stream(reader) == content[index]);
  }
  fails(OPERATION_STREAM_READ, reader, 0, 0, 0);
  close_stream(reader);
  unlink(path);
}

static void test_clock_and_random(void) {
  uint64_t previous = succeeds(OPERATION_CLOCK_MONOTONIC, 0, 0, 0, 0);
  for (int index = 0; index < 32; index++) {
    uint64_t current = succeeds(OPERATION_CLOCK_MONOTONIC, 0, 0, 0, 0);
    assert(current >= previous);
    previous = current;
  }
  assert(succeeds(OPERATION_CLOCK_REALTIME, 0, 0, 0, 0) > 0);
  assert(succeeds(OPERATION_CLOCK_REALTIME, 0, 0, 0, 0) > 0);

  unsigned char first[RANDOM_SAMPLE_SIZE];
  unsigned char second[RANDOM_SAMPLE_SIZE];
  for (size_t index = 0; index < RANDOM_SAMPLE_SIZE; index++) {
    first[index] = (unsigned char)succeeds(OPERATION_RANDOM_BYTE, 0, 0, 0, 0);
    second[index] = (unsigned char)succeeds(OPERATION_RANDOM_BYTE, 0, 0, 0, 0);
  }
  assert(memcmp(first, second, RANDOM_SAMPLE_SIZE) != 0);
}

static void test_unknown_operations(void) {
  fails(0x0001, 0, 0, 0, 0);
  fails(0x0002, 0, 0, 0, 0);
  fails(0x0104, 0, 0, 0, 0);
  fails(0x0205, 0, 0, 0, 0);
  fails(0x0302, 0, 0, 0, 0);
  fails(0x0401, 0, 0, 0, 0);
  fails(0x8000, 0, 0, 0, 0);
  fails(0xFFFF, 0, 0, 0, 0);
}

static void check_argument_stream(void) {
  const unsigned char expected[] = "argument-stream\0expected\0";
  for (size_t index = 0; index < sizeof expected - 1; index++) {
    assert(read_stream(STREAM_ARGUMENTS) == expected[index]);
  }
  fails(OPERATION_STREAM_READ, STREAM_ARGUMENTS, 0, 0, 0);
  flush_stream(STREAM_ARGUMENTS);
  fails(OPERATION_STREAM_WRITE, STREAM_ARGUMENTS, 'x', 0, 0);
}

static void test_argument_stream(void) {
  pid_t child = fork();
  assert(child >= 0);
  if (child == 0) {
    char *child_arguments[] = {program_path, "argument-stream", "expected",
                               NULL};
    execvp(program_path, child_arguments);
    _exit(101);
  }
  int status = 0;
  assert(waitpid(child, &status, 0) == child);
  assert(WIFEXITED(status));
  assert(WEXITSTATUS(status) == 0);
}

int main(int argument_count, char **argument_values) {
  if (argument_count == 3 &&
      strcmp(argument_values[1], "argument-stream") == 0 &&
      strcmp(argument_values[2], "expected") == 0) {
    assert(lap_runtime_start(argument_count, argument_values));
    check_argument_stream();
    lap_runtime_finish();
    return 0;
  }

  program_path = argument_values[0];
  assert(lap_runtime_start(argument_count, argument_values));

  test_memory_operations();
  test_process_exit_flushes();
  test_standard_input();
  test_standard_output_buffering();
  test_standard_error_unbuffered();
  test_file_streams();
  test_stream_path_window();
  test_independent_stream_buffers();
  test_stream_table_growth();
  test_stream_refill_reading();
  test_flush_failure_discards_buffer();
  test_clock_and_random();
  test_unknown_operations();
  test_argument_stream();

  lap_runtime_finish();
  return 0;
}
