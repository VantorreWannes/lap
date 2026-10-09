#ifdef _WIN32
#define _CRT_RAND_S
#endif

#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#ifdef _WIN32
#include <io.h>
#endif

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
  OPERATION_STREAM_READ_BLOCK = 0x0205,
  OPERATION_STREAM_WRITE_BLOCK = 0x0206,
  OPERATION_CLOCK_MONOTONIC = 0x0300,
  OPERATION_CLOCK_REALTIME = 0x0301,
  OPERATION_RANDOM_BYTE = 0x0400,
};

enum {
  STREAM_MODE_READ = 0,
  STREAM_MODE_WRITE = 1,
  STREAM_MODE_APPEND = 2,
};

enum {
  STREAM_BUFFER_CAPACITY = 4096,
  RANDOM_BUFFER_CAPACITY = 256,
  INITIAL_TABLE_CAPACITY = 16,
};

struct memory_entry {
  unsigned char *bytes;
  uint64_t length;
  bool live;
};

struct stream_entry {
  int descriptor;
  bool live;
  bool readable;
  bool writable;
  bool unbuffered;
  unsigned char *write_bytes;
  uint64_t write_length;
  uint64_t write_capacity;
  unsigned char *read_bytes;
  uint64_t read_offset;
  uint64_t read_length;
  uint64_t read_capacity;
};

static struct memory_entry *memory_entries;
static uint64_t memory_entry_count;
static uint64_t memory_entry_capacity;

static struct stream_entry *stream_entries;
static uint64_t stream_entry_count;
static uint64_t stream_entry_capacity;

static uint64_t monotonic_last;
static bool monotonic_initialized;

#ifndef _WIN32
static int random_descriptor = -1;
#endif
static unsigned char random_bytes[RANDOM_BUFFER_CAPACITY];
static uint64_t random_offset;
static uint64_t random_length;

static struct memory_entry *memory_table_append(void) {
  if (memory_entry_count == memory_entry_capacity) {
    uint64_t capacity = memory_entry_capacity == 0 ? INITIAL_TABLE_CAPACITY
                                                   : memory_entry_capacity * 2;
    struct memory_entry *entries =
        realloc(memory_entries, capacity * sizeof *entries);
    if (entries == NULL) {
      return NULL;
    }
    memory_entries = entries;
    memory_entry_capacity = capacity;
  }
  struct memory_entry *entry = &memory_entries[memory_entry_count];
  entry->bytes = NULL;
  entry->length = 0;
  entry->live = false;
  memory_entry_count++;
  return entry;
}

static struct memory_entry *memory_entry_for_handle(uint64_t handle) {
  if (handle >= memory_entry_count) {
    return NULL;
  }
  struct memory_entry *entry = &memory_entries[handle];
  if (!entry->live) {
    return NULL;
  }
  return entry;
}

static bool memory_acquire(uint64_t length, uint64_t *handle) {
  if (length != (uint64_t)(size_t)length) {
    return false;
  }
  unsigned char *bytes = malloc(length == 0 ? 1 : length);
  if (bytes == NULL) {
    return false;
  }
  struct memory_entry *entry = memory_table_append();
  if (entry == NULL) {
    free(bytes);
    return false;
  }
  entry->bytes = bytes;
  entry->length = length;
  entry->live = true;
  *handle = (uint64_t)(entry - memory_entries);
  return true;
}

static bool memory_release(uint64_t handle) {
  struct memory_entry *entry = memory_entry_for_handle(handle);
  if (entry == NULL) {
    return false;
  }
  free(entry->bytes);
  entry->bytes = NULL;
  entry->length = 0;
  entry->live = false;
  return true;
}

static bool memory_read(uint64_t handle, uint64_t offset, unsigned char *byte) {
  struct memory_entry *entry = memory_entry_for_handle(handle);
  if (entry == NULL || offset >= entry->length) {
    return false;
  }
  *byte = entry->bytes[offset];
  return true;
}

static bool memory_write(uint64_t handle, uint64_t offset, unsigned char byte) {
  struct memory_entry *entry = memory_entry_for_handle(handle);
  if (entry == NULL || offset >= entry->length) {
    return false;
  }
  entry->bytes[offset] = byte;
  return true;
}

static struct stream_entry *stream_table_append(void) {
  if (stream_entry_count == stream_entry_capacity) {
    uint64_t capacity = stream_entry_capacity == 0 ? INITIAL_TABLE_CAPACITY
                                                   : stream_entry_capacity * 2;
    struct stream_entry *entries =
        realloc(stream_entries, capacity * sizeof *entries);
    if (entries == NULL) {
      return NULL;
    }
    stream_entries = entries;
    stream_entry_capacity = capacity;
  }
  struct stream_entry *entry = &stream_entries[stream_entry_count];
  memset(entry, 0, sizeof *entry);
  entry->descriptor = -1;
  stream_entry_count++;
  return entry;
}

static struct stream_entry *stream_entry_for_handle(uint64_t handle) {
  if (handle >= stream_entry_count) {
    return NULL;
  }
  struct stream_entry *entry = &stream_entries[handle];
  if (!entry->live) {
    return NULL;
  }
  return entry;
}

static bool stream_flush(struct stream_entry *stream) {
  if (!stream->writable || stream->unbuffered) {
    return true;
  }
  uint64_t written = 0;
  while (written < stream->write_length) {
    ssize_t result = write(stream->descriptor, stream->write_bytes + written,
                           stream->write_length - written);
    if (result < 0) {
      if (errno == EINTR) {
        continue;
      }
      stream->write_length = 0;
      return false;
    }
    if (result == 0) {
      stream->write_length = 0;
      return false;
    }
    written += (uint64_t)result;
  }
  stream->write_length = 0;
  return true;
}

static bool stream_write_byte(struct stream_entry *stream, unsigned char byte) {
  if (stream->unbuffered) {
    ssize_t result;
    do {
      result = write(stream->descriptor, &byte, 1);
    } while (result < 0 && errno == EINTR);
    return result == 1;
  }
  if (stream->write_length == stream->write_capacity) {
    if (!stream_flush(stream)) {
      return false;
    }
  }
  stream->write_bytes[stream->write_length] = byte;
  stream->write_length++;
  return true;
}

static bool stream_read_byte(struct stream_entry *stream, unsigned char *byte) {
  if (stream->read_offset < stream->read_length) {
    *byte = stream->read_bytes[stream->read_offset];
    stream->read_offset++;
    return true;
  }
  if (stream->descriptor < 0) {
    return false;
  }
  ssize_t result;
  do {
    result =
        read(stream->descriptor, stream->read_bytes, stream->read_capacity);
  } while (result < 0 && errno == EINTR);
  if (result <= 0) {
    return false;
  }
  stream->read_offset = 0;
  stream->read_length = (uint64_t)result;
  *byte = stream->read_bytes[stream->read_offset];
  stream->read_offset++;
  return true;
}

static bool stream_read_block(struct stream_entry *stream, uint64_t handle,
                              uint64_t offset, uint64_t length,
                              uint64_t *count) {
  struct memory_entry *block = memory_entry_for_handle(handle);
  if (block == NULL || offset > block->length ||
      length > block->length - offset) {
    return false;
  }
  uint64_t total = 0;
  while (total < length) {
    if (stream->read_offset < stream->read_length) {
      uint64_t available = stream->read_length - stream->read_offset;
      uint64_t wanted = length - total;
      uint64_t taken = available < wanted ? available : wanted;
      memcpy(block->bytes + offset + total,
             stream->read_bytes + stream->read_offset, taken);
      stream->read_offset += taken;
      total += taken;
      continue;
    }
    if (stream->descriptor < 0) {
      break;
    }
    ssize_t result;
    do {
      result = read(stream->descriptor, block->bytes + offset + total,
                    length - total);
    } while (result < 0 && errno == EINTR);
    if (result <= 0) {
      break;
    }
    total += (uint64_t)result;
  }
  *count = total;
  return total > 0 || length == 0;
}

static bool stream_write_block(struct stream_entry *stream, uint64_t handle,
                               uint64_t offset, uint64_t length) {
  struct memory_entry *block = memory_entry_for_handle(handle);
  if (block == NULL || offset > block->length ||
      length > block->length - offset) {
    return false;
  }
  const unsigned char *source = block->bytes + offset;
  uint64_t written = 0;
  while (written < length) {
    if (stream->unbuffered) {
      ssize_t result;
      do {
        result = write(stream->descriptor, source + written, length - written);
      } while (result < 0 && errno == EINTR);
      if (result <= 0) {
        return false;
      }
      written += (uint64_t)result;
      continue;
    }
    if (stream->write_length == stream->write_capacity) {
      if (!stream_flush(stream)) {
        return false;
      }
    }
    uint64_t space = stream->write_capacity - stream->write_length;
    uint64_t wanted = length - written;
    uint64_t moved = space < wanted ? space : wanted;
    memcpy(stream->write_bytes + stream->write_length, source + written, moved);
    stream->write_length += moved;
    written += moved;
  }
  return true;
}

static bool stream_open(uint64_t path_handle, uint64_t path_offset,
                        uint64_t path_length, uint64_t mode, uint64_t *handle) {
  struct memory_entry *block = memory_entry_for_handle(path_handle);
  if (block == NULL || path_offset > block->length ||
      path_length > block->length - path_offset) {
    return false;
  }
  int flags;
  if (mode == STREAM_MODE_READ) {
    flags = O_RDONLY;
  } else if (mode == STREAM_MODE_WRITE) {
    flags = O_WRONLY | O_CREAT | O_TRUNC;
  } else if (mode == STREAM_MODE_APPEND) {
    flags = O_WRONLY | O_CREAT | O_APPEND;
  } else {
    return false;
  }
#ifdef _WIN32
  flags |= O_BINARY;
#endif
  char *path = malloc(path_length + 1);
  if (path == NULL) {
    return false;
  }
  memcpy(path, block->bytes + path_offset, path_length);
  path[path_length] = '\0';
  int descriptor = open(path, flags, 0666);
  free(path);
  if (descriptor < 0) {
    return false;
  }
  unsigned char *buffer = malloc(STREAM_BUFFER_CAPACITY);
  if (buffer == NULL) {
    close(descriptor);
    return false;
  }
  struct stream_entry *stream = stream_table_append();
  if (stream == NULL) {
    free(buffer);
    close(descriptor);
    return false;
  }
  stream->descriptor = descriptor;
  stream->readable = mode == STREAM_MODE_READ;
  stream->writable = mode != STREAM_MODE_READ;
  if (stream->readable) {
    stream->read_bytes = buffer;
    stream->read_capacity = STREAM_BUFFER_CAPACITY;
  } else {
    stream->write_bytes = buffer;
    stream->write_capacity = STREAM_BUFFER_CAPACITY;
  }
  stream->live = true;
  *handle = (uint64_t)(stream - stream_entries);
  return true;
}

static bool stream_close(struct stream_entry *stream) {
  bool flushed = stream_flush(stream);
  if (stream->descriptor >= 0) {
    close(stream->descriptor);
  }
  free(stream->write_bytes);
  free(stream->read_bytes);
  stream->descriptor = -1;
  stream->write_bytes = NULL;
  stream->write_length = 0;
  stream->write_capacity = 0;
  stream->read_bytes = NULL;
  stream->read_offset = 0;
  stream->read_length = 0;
  stream->read_capacity = 0;
  stream->live = false;
  return flushed;
}

static void flush_all_streams(void) {
  for (uint64_t index = 0; index < stream_entry_count; index++) {
    if (stream_entries[index].live) {
      stream_flush(&stream_entries[index]);
    }
  }
}

static bool clock_monotonic(uint64_t *nanoseconds) {
  struct timespec now;
  if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) {
    return false;
  }
  uint64_t value = (uint64_t)now.tv_sec * 1000000000ULL + (uint64_t)now.tv_nsec;
  if (monotonic_initialized && value < monotonic_last) {
    value = monotonic_last;
  }
  monotonic_last = value;
  monotonic_initialized = true;
  *nanoseconds = value;
  return true;
}

static bool clock_realtime(uint64_t *nanoseconds) {
  struct timespec now;
  if (clock_gettime(CLOCK_REALTIME, &now) != 0) {
    return false;
  }
  *nanoseconds = (uint64_t)now.tv_sec * 1000000000ULL + (uint64_t)now.tv_nsec;
  return true;
}

static uint64_t random_fill(unsigned char *bytes, size_t capacity) {
#ifdef _WIN32
  for (size_t index = 0; index < capacity; index++) {
    unsigned int value;
    if (rand_s(&value) != 0) {
      return 0;
    }
    bytes[index] = (unsigned char)(value & 0xFF);
  }
  return (uint64_t)capacity;
#else
  if (random_descriptor < 0) {
    random_descriptor = open("/dev/urandom", O_RDONLY);
    if (random_descriptor < 0) {
      return 0;
    }
  }
  ssize_t result;
  do {
    result = read(random_descriptor, bytes, capacity);
  } while (result < 0 && errno == EINTR);
  if (result <= 0) {
    return 0;
  }
  return (uint64_t)result;
#endif
}

static bool random_byte(unsigned char *byte) {
  if (random_offset == random_length) {
    uint64_t length = random_fill(random_bytes, sizeof random_bytes);
    if (length == 0) {
      return false;
    }
    random_offset = 0;
    random_length = length;
  }
  *byte = random_bytes[random_offset];
  random_offset++;
  return true;
}

static void report_success(uint64_t *out, uint64_t payload) {
  out[0] = 1;
  out[1] = payload;
}

static bool open_standard_stream(int descriptor, bool readable, bool writable,
                                 bool unbuffered) {
  struct stream_entry *stream = stream_table_append();
  if (stream == NULL) {
    return false;
  }
  stream->descriptor = descriptor;
  stream->readable = readable;
  stream->writable = writable;
  stream->unbuffered = unbuffered;
  if (readable) {
    stream->read_bytes = malloc(STREAM_BUFFER_CAPACITY);
    if (stream->read_bytes == NULL) {
      return false;
    }
    stream->read_capacity = STREAM_BUFFER_CAPACITY;
  }
  if (writable && !unbuffered) {
    stream->write_bytes = malloc(STREAM_BUFFER_CAPACITY);
    if (stream->write_bytes == NULL) {
      free(stream->read_bytes);
      stream->read_bytes = NULL;
      return false;
    }
    stream->write_capacity = STREAM_BUFFER_CAPACITY;
  }
  stream->live = true;
  return true;
}

static bool open_argument_stream(int argument_count, char **argument_values) {
  uint64_t total_length = 0;
  for (int index = 1; index < argument_count; index++) {
    total_length += (uint64_t)strlen(argument_values[index]) + 1;
  }
  struct stream_entry *stream = stream_table_append();
  if (stream == NULL) {
    return false;
  }
  stream->read_bytes = malloc(total_length == 0 ? 1 : total_length);
  if (stream->read_bytes == NULL) {
    return false;
  }
  uint64_t offset = 0;
  for (int index = 1; index < argument_count; index++) {
    size_t length = strlen(argument_values[index]);
    memcpy(stream->read_bytes + offset, argument_values[index], length);
    offset += (uint64_t)length;
    stream->read_bytes[offset] = 0;
    offset++;
  }
  stream->readable = true;
  stream->read_length = total_length;
  stream->read_capacity = total_length;
  stream->live = true;
  return true;
}

bool lap_runtime_start(int argument_count, char **argument_values) {
#ifdef _WIN32
  _setmode(0, O_BINARY);
  _setmode(1, O_BINARY);
  _setmode(2, O_BINARY);
#endif
  if (!open_standard_stream(0, true, false, false)) {
    return false;
  }
  if (!open_standard_stream(1, false, true, false)) {
    return false;
  }
  if (!open_standard_stream(2, false, true, true)) {
    return false;
  }
  if (!open_argument_stream(argument_count, argument_values)) {
    return false;
  }
  return true;
}

void lap_runtime_finish(void) { flush_all_streams(); }

void lap_extern_process_exit(uint64_t status, uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  flush_all_streams();
  exit((int)(status & 0xFF));
}

void lap_extern_memory_acquire(uint64_t length, uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  uint64_t handle;
  if (memory_acquire(length, &handle)) {
    report_success(out, handle);
  }
}

void lap_extern_memory_release(uint64_t handle, uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  if (memory_release(handle)) {
    report_success(out, 0);
  }
}

void lap_extern_memory_read(uint64_t handle, uint64_t offset, uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  unsigned char byte;
  if (memory_read(handle, offset, &byte)) {
    report_success(out, (uint64_t)byte);
  }
}

void lap_extern_memory_write(uint64_t handle, uint64_t offset, uint64_t byte,
                             uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  if (memory_write(handle, offset, (unsigned char)(byte & 0xFF))) {
    report_success(out, 0);
  }
}

void lap_extern_stream_open(uint64_t path_handle, uint64_t path_offset,
                            uint64_t path_length, uint64_t mode,
                            uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  uint64_t handle;
  if (stream_open(path_handle, path_offset, path_length, mode, &handle)) {
    report_success(out, handle);
  }
}

void lap_extern_stream_read(uint64_t handle, uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  struct stream_entry *stream = stream_entry_for_handle(handle);
  unsigned char byte;
  if (stream != NULL && stream->readable && stream_read_byte(stream, &byte)) {
    report_success(out, (uint64_t)byte);
  }
}

void lap_extern_stream_write(uint64_t handle, uint64_t byte, uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  struct stream_entry *stream = stream_entry_for_handle(handle);
  if (stream != NULL && stream->writable &&
      stream_write_byte(stream, (unsigned char)(byte & 0xFF))) {
    report_success(out, 0);
  }
}

void lap_extern_stream_flush(uint64_t handle, uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  struct stream_entry *stream = stream_entry_for_handle(handle);
  if (stream != NULL && stream_flush(stream)) {
    report_success(out, 0);
  }
}

void lap_extern_stream_close(uint64_t handle, uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  struct stream_entry *stream = stream_entry_for_handle(handle);
  if (stream != NULL && stream_close(stream)) {
    report_success(out, 0);
  }
}

void lap_extern_stream_read_block(uint64_t handle, uint64_t block_handle,
                                  uint64_t offset, uint64_t length,
                                  uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  struct stream_entry *stream = stream_entry_for_handle(handle);
  uint64_t count;
  if (stream != NULL && stream->readable &&
      stream_read_block(stream, block_handle, offset, length, &count)) {
    report_success(out, count);
  }
}

void lap_extern_stream_write_block(uint64_t handle, uint64_t block_handle,
                                   uint64_t offset, uint64_t length,
                                   uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  struct stream_entry *stream = stream_entry_for_handle(handle);
  if (stream != NULL && stream->writable &&
      stream_write_block(stream, block_handle, offset, length)) {
    report_success(out, 0);
  }
}

void lap_extern_clock_monotonic(uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  uint64_t nanoseconds;
  if (clock_monotonic(&nanoseconds)) {
    report_success(out, nanoseconds);
  }
}

void lap_extern_clock_realtime(uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  uint64_t nanoseconds;
  if (clock_realtime(&nanoseconds)) {
    report_success(out, nanoseconds);
  }
}

void lap_extern_random_byte(uint64_t *out) {
  out[0] = 0;
  out[1] = 0;
  unsigned char byte;
  if (random_byte(&byte)) {
    report_success(out, (uint64_t)byte);
  }
}

void lap_extern(uint64_t operation, uint64_t argument0, uint64_t argument1,
                uint64_t argument2, uint64_t argument3, uint64_t *out) {
  switch (operation) {
  case OPERATION_PROCESS_EXIT:
    lap_extern_process_exit(argument0, out);
    break;
  case OPERATION_MEMORY_ACQUIRE:
    lap_extern_memory_acquire(argument0, out);
    break;
  case OPERATION_MEMORY_RELEASE:
    lap_extern_memory_release(argument0, out);
    break;
  case OPERATION_MEMORY_READ:
    lap_extern_memory_read(argument0, argument1, out);
    break;
  case OPERATION_MEMORY_WRITE:
    lap_extern_memory_write(argument0, argument1, argument2, out);
    break;
  case OPERATION_STREAM_OPEN:
    lap_extern_stream_open(argument0, argument1, argument2, argument3, out);
    break;
  case OPERATION_STREAM_READ:
    lap_extern_stream_read(argument0, out);
    break;
  case OPERATION_STREAM_WRITE:
    lap_extern_stream_write(argument0, argument1, out);
    break;
  case OPERATION_STREAM_FLUSH:
    lap_extern_stream_flush(argument0, out);
    break;
  case OPERATION_STREAM_CLOSE:
    lap_extern_stream_close(argument0, out);
    break;
  case OPERATION_STREAM_READ_BLOCK:
    lap_extern_stream_read_block(argument0, argument1, argument2, argument3, out);
    break;
  case OPERATION_STREAM_WRITE_BLOCK:
    lap_extern_stream_write_block(argument0, argument1, argument2, argument3, out);
    break;
  case OPERATION_CLOCK_MONOTONIC:
    lap_extern_clock_monotonic(out);
    break;
  case OPERATION_CLOCK_REALTIME:
    lap_extern_clock_realtime(out);
    break;
  case OPERATION_RANDOM_BYTE:
    lap_extern_random_byte(out);
    break;
  default:
    out[0] = 0;
    out[1] = 0;
    break;
  }
}
