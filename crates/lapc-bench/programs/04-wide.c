#include "bench.h"

typedef unsigned __int128 u128;

static u128 pack(uint64_t low, uint64_t high) {
  return ((u128)high << 64) | (u128)low;
}

static uint64_t loop(uint64_t count, uint64_t i, uint64_t acc) {
  while (i != count) {
    uint64_t next = i + 1;
    u128 pair = pack(acc, i);
    uint64_t low = (uint64_t)pair;
    uint64_t high = (uint64_t)(pair >> 64);
    if ((i & 1) == 0) {
      acc = (low + high) ^ acc;
    } else {
      acc = (low ^ high) + acc;
    }
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
