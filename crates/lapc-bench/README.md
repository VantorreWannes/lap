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
| `00-baseline` | 3535800 | 3507200 | 1.01 |
| `01-inlining` | 5270100 | 3453700 | 1.53 |
| `02-mem2reg` | 40157100 | 42029700 | 0.96 |
| `03-branch` | 17261900 | 17105600 | 1.01 |
| `04-wide` | 6796700 | 6956800 | 0.98 |
| `05-masks` | 24333900 | 17257500 | 1.41 |

## Ownership

This crate owns the benchmark programs and the measurement protocol. `lapc-codegen` owns emitting C and finding the C compiler; `lapc-driver` owns building a Lap program.
