#include "bench.h"

static uint64_t mix(uint64_t i, uint64_t acc) {
  uint64_t a = i + acc;
  uint64_t b = a ^ i;
  uint64_t c = b + a;
  uint64_t d = c ^ b;
  uint64_t e = d + c;
  uint64_t f = e ^ d;
  uint64_t g = f + e;
  uint64_t h = g ^ f;
  return h + g;
}

static uint64_t loop(uint64_t count, uint64_t i, uint64_t acc) {
  while (i != count) {
    uint64_t next = i + 1;
    acc = mix(i, acc);
    i = next;
  }
  return acc;
}

int main(void) {
  uint64_t start = bench_start();
  uint64_t result = loop(16777216ULL, 0, 0);
  uint64_t elapsed = bench_start() - start;
  bench_write(elapsed);
  bench_write(result);
  return 0;
}
