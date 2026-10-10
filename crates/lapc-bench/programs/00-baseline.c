#include "bench.h"

static uint64_t sum_loop(uint64_t count, uint64_t i, uint64_t acc) {
  while (i != count) {
    uint64_t next = i + 1;
    if ((i & 1) == 0) {
      acc = acc + i;
    } else {
      acc = acc ^ i;
    }
    i = next;
  }
  return acc;
}

int main(void) {
  uint64_t start = bench_start();
  uint64_t result = sum_loop(16777216ULL, 0, 0);
  uint64_t elapsed = bench_start() - start;
  bench_write(elapsed);
  bench_write(result);
  return 0;
}
