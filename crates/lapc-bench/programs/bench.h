#include <stdint.h>
#include <stdio.h>
#include <time.h>

static uint64_t bench_start(void) {
  struct timespec now;
  clock_gettime(CLOCK_MONOTONIC, &now);
  return (uint64_t)now.tv_sec * 1000000000ULL + (uint64_t)now.tv_nsec;
}

static void bench_write(uint64_t value) {
  fwrite(&value, sizeof value, 1, stdout);
}
