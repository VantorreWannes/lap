#include "bench.h"

static uint8_t byte_mix(uint8_t acc) {
  uint8_t a = (uint8_t)(acc + 1);
  uint8_t b = (uint8_t)(a ^ acc);
  uint8_t c = (uint8_t)(b + a);
  uint8_t d = (uint8_t)(c ^ b);
  return (uint8_t)(d + c);
}

static uint8_t loop(uint64_t count, uint64_t i, uint8_t acc) {
  while (i != count) {
    uint64_t next = i + 1;
    acc = byte_mix(acc);
    i = next;
  }
  return acc;
}

int main(void) {
  uint64_t start = bench_start();
  uint8_t result = loop(16777216ULL, 0, 0);
  uint64_t elapsed = bench_start() - start;
  bench_write(elapsed);
  fwrite(&result, 1, 1, stdout);
  return 0;
}
