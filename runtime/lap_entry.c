#include <stdbool.h>
#include <stdint.h>

extern uint64_t lap_main(void);
extern bool lap_runtime_start(int argument_count, char **argument_values);
extern void lap_runtime_finish(void);

int main(int argument_count, char **argument_values) {
  if (!lap_runtime_start(argument_count, argument_values)) {
    return 1;
  }
  uint64_t status = lap_main();
  lap_runtime_finish();
  return (int)(status & 1);
}
