# Lables and Primitives

A minimal, deterministic programming language built on a single hardware-level premise: **everything is a `BIT` or an array of `BIT`s.**

There is no heap, no pointer types as values, no dynamic allocation, and no integer assumptions baked into the syntax. All logic scales from bare `NAND` gates up through nested bit arrays.

## Features

- **5 Primitives:** `BIT`, `NAND`, `BRANCH`, `SYS`, and `*`.
- **Structural Symmetry:** Arrays are built and deconstructed exclusively with `[...]`.
- **In-Place Mutation:** References (`*`) exist only for downward in-place mutation; they cannot escape the stack, preventing dangling pointers by construction.
- **No Invisible Layouts:** No implicit `usize`, no endianness assumptions, and no array-stride type machinery.

## Quick Example

```text
// Logic from scratch
ON     = (x: BIT) BIT { NAND(x, NAND(x, x)) }
OFF    = (x: BIT) BIT { NAND(ON(x), ON(x)) }
u1.not = (x: BIT) BIT { NAND(x, x) }

// Composite types
u2 = [BIT, BIT]

// In-place mutation using reference unbinding
invert_pair = (p: *u2) BIT {
    [*lo, *hi] = p
    lo = u1.not(lo)
    hi = u1.not(hi)
    OFF(0)
}

main = () BIT {
    pair = [ON(0), OFF(0)]
    invert_pair(pair)          // 'pair' is now [0, 1]
    [a, b] = pair
    b                          // returns 1
}

See [SPECIFICATION.md](SPECIFICATION.md) for the complete language specification.