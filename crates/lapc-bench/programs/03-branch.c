#include "bench.h"

static uint64_t choose(uint64_t i, uint64_t acc) {
  uint64_t step;
  if ((i & 1) == 0) {
    step = acc + i;
  } else {
    step = acc ^ i;
  }
  if (step < 1048576ULL) {
    return step + 1;
  }
  return step - 1;
}

static uint64_t loop(uint64_t count, uint64_t i, uint64_t acc) {
  while (i != count) {
    uint64_t next = i + 1;
    acc = choose(i, acc);
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
