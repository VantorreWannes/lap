# Labels and Primitives

Labels and Primitives is a minimal, deterministic systems programming language built on a single hardware-level premise: **everything is a `BIT` or an array of `BIT`s.**

There is no heap, no allocator, no implicit integer types, and no hidden memory layout. Computation scales from raw universal gates (`NAND`) to nested bit arrays. Standard software operations defined in `STD` can be automatically hoisted by the compiler into native CPU instructions.

## Key Ideas

- **Six Primitives:** `BIT`, `NAND`, `BRANCH`, `SYS`, `STD`, and `*`.
- **Pure Structural Layout:** Memory consists solely of bits and arrays defined with brackets (`[...]`). Arrays are unpacked using the same structural syntax used to build them.
- **No Literal Numbers:** The grammar contains no decimal, hexadecimal, or scalar number literals. The ground and power states of a bit are defined directly as `BIT.zero` and `BIT.one`.
- **Downward References (`*`):** References exist solely to pass mutable views of existing memory down the call stack. Because references cannot be returned, stored in global memory, or stored into heap structures, dangling pointers and memory leaks are structurally impossible.
- **Hierarchical Dot Paths (`.`):** Periods only ever exist between two labels, acting as an infix scope delimiter. Chaining is fully supported (e.g., `STD.u8.add`), allowing operations and types to be grouped directly under what contains them. There are no classes, receivers, or access modifiers—anyone can call any path directly.
- **Transparent Compilation:** Operations in `STD` are defined from raw gates. When compiling for targets with dedicated hardware instructions (such as an ALU), the compiler hoists canonical paths (like `STD.u8.not`) directly into single machine instructions.

## Code Example

```text
zero = BIT.zero
one = BIT.one

// Standard 8-bit byte from the STD primitive
u8 = STD.u8

// Invert a byte in place using a downward reference
invert = (val: *u8) BIT {
    val = STD.u8.not(val)
    zero
}

main = () BIT {
    byte = [zero, one, zero, one, zero, one, zero, one]
    invert(byte) // hoisted to a native NOT instruction

    // Deconstruct byte into its individual bits
    [b0, b1, b2, b3, b4, b5, b6, b7] = byte
    b1
}
```

## How It Works

### Uniform Storage

All data is concrete and composed of `BIT`. A composite word is simply an array of bits. Multi-bit values are constructed explicitly from arrays of bit constants (`BIT.zero` and `BIT.one`).

### Dot Semantics and Hierarchical Scoping

A period (`.`) is strictly an infix delimiter that sits between two labels (`Left.Right`). Dots can be chained to express deeper containment (e.g., `STD.u8.add`). 

Each segment is just a label scoped under the preceding label. There is no object-oriented dispatch, `this`/`self` hidden arguments, or visibility restriction (no `public`/`private`); any path can be referenced and called directly from anywhere.

### In-Place Mutation

Passing a variable to a function by value copies its bits. Prefixing a parameter or binding pattern with `*` creates a zero-overhead reference to the caller's storage slot. Any assignment to that reference writes through to the original location.

### The `STD` Library and Hardware Hoisting

The `STD` primitive provides a baseline library of types, constants, and operations built from pure gates. On platforms where hardware support exists, the compiler recognizes canonical labels (such as `STD.u32.add` or `STD.u64.mul`) and emits the target CPU's native assembly instructions instead of expanding the gate network.

### System Calls

Programs interact with the operating system kernel directly via the `SYS` primitive, which binds to host system call conventions without requiring a C runtime.