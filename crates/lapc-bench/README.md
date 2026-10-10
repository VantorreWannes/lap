# lapc-bench

Measures the runtime of Lap programs against equivalent C programs.

## Scope

This crate answers one question: how fast is a Lap program, compared to the same program written in C.

It does not measure the compiler.

## Running

```
cargo run --release -p lapc-bench
```

Every `programs/*.lap` is built with `lapc`, and every `programs/*.c` is built with the C compiler the driver finds. Each program is run five times, and the fastest run is reported.

## Protocol

A benchmark writes the elapsed nanoseconds as a little-endian `u64`, then its result bytes, then exits with status 0. Each program times itself with `clock.monotonic`, so process startup is not measured.

The harness compares the result bytes of the Lap program with the C program. A benchmark that computes a different answer fails instead of reporting a time.

## Benchmarks

| Benchmark | Isolates |
| --- | --- |
| `00-baseline` | a tail-recursive loop over `U64` intrinsics with a branch |
| `01-inlining` | a call to a small function whose body holds a binding |
| `02-mem2reg` | a chain of local bindings read back within one call |
| `03-branch` | a branch whose result feeds a second branch |
| `04-wide` | a `U128` value built, passed, and destructured in a loop |
| `05-masks` | a chain of `U8` intrinsics |

## Recorded run

Windows, x86_64, clang 21.1.0 (zig), `-O2`, 16777216 iterations. Nanoseconds, fastest of five. A snapshot, not a contract.

| Benchmark | Lap | C | Lap/C |
| --- | --- | --- | --- |
| `00-baseline` | 8725100 | 3471300 | 2.51 |
| `01-inlining` | 9032400 | 3395900 | 2.66 |
| `02-mem2reg` | 30259600 | 40177700 | 0.75 |
| `03-branch` | 16458000 | 16793300 | 0.98 |
| `04-wide` | 8656500 | 7001300 | 1.24 |
| `05-masks` | 33506300 | 16727700 | 2.00 |

The same run with Cranelift `opt_level` at its default of `none`:

| Benchmark | Lap | C | Lap/C |
| --- | --- | --- | --- |
| `00-baseline` | 15850300 | 3489600 | 4.54 |
| `01-inlining` | 14579500 | 3365400 | 4.33 |
| `02-mem2reg` | 30747300 | 40512600 | 0.76 |
| `03-branch` | 21249000 | 16895500 | 1.26 |
| `04-wide` | 11242600 | 6817700 | 1.65 |
| `05-masks` | 40402100 | 16786000 | 2.41 |

## Ownership

This crate owns the benchmark programs and the measurement protocol. `lapc-driver` owns building a Lap program and finding the C compiler.
