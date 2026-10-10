# lapc

`lapc` is a compiler for Lap. This document records the choices it makes that the language does not.

## Scope

[README.md](README.md) owns the language. This document owns the lowering, the intrinsic table, the backend, and the runtime interface. [EXTERN.md](EXTERN.md) owns the external operation table.

`lapc` adds no syntax. It accepts one form the language does not name: an erased function declaration, which [Operation erasure](#operation-erasure) owns. Every other program it accepts is accepted by any conforming compiler.

## Input

`lapc` takes one or more files and appends them in the order given, top to bottom. The result is one program.

`lapc check` checks a program. `lapc build` emits C and compiles it with the runtime using a C compiler. `CC` names the compiler when set; otherwise the driver tries `cc`, `gcc`, `clang`, and `zig cc`, in that order, and uses the first that can compile the runtime. The runtime needs a POSIX-style C compiler: on Windows, a MinGW-w64 toolchain such as zig or MSYS2, not MSVC. `LAPC_DUMP_C` names a file to write the generated C to, for debugging.

## Structure

`lapc` is a workspace of libraries and one binary. Each stage is a crate: `lapc-ast`, `lapc-parse`, `lapc-ir`, `lapc-check`, `lapc-erase`, `lapc-codegen`, and `lapc-driver`. A stage is swapped by swapping its crate, not by replacing a binary.

## Operation erasure

Every operation in Lap is a `NAND` tree, so arithmetic written as a library is slow unless the compiler recognizes it. Erasure is that recognition: a function whose label is in the table is replaced by a native operation.

This is why the language does not need arithmetic primitives.

### Table

Each entry names an intrinsic and its lowering. The label is the contract: a function with this label must behave as the intrinsic.

```
operation liblapc.u8.add:
    lowering: iadd
```

The table is hardcoded. The compiler does not discover entries. A label is matched whole, including its `liblapc.` prefix; there is no suffix or pattern matching.

### Declaring

A function whose label is in the table is declared with an empty collection in place of a signature and a body:

```
liblapc.u8.add = []
```

The compiler derives the signature from the label. The segment after `liblapc.` names the operand type, and the table entry names the arity and the result. The erasure pass then replaces the body.

A label outside the table keeps the language rule: `label = []` binds the empty collection type.

### Matching

1. Look up the function's label in the table.
2. On a hit, replace the function with the intrinsic node.

An intrinsic is a first-class IR node, not an `EXTERN` call, so later passes still see it. Erasure never introduces an `EXTERN` call, and never removes one.

### Contracts

- Matching is by label.
- Only a whole function matches. Erasure runs before inlining.
- A miss is never an error. The `NAND` tree is correct, only slower.
- Erasure applies to values of 64 bits or fewer. A wider function is left as a `NAND` tree.
- A comparison (`eq`, `lt`, `is.zero`) takes operands of one type and yields a `BIT`. Every other intrinsic yields its operand type.
- A division (`div.mod`) takes two operands of one type and yields their quotient and remainder as a pair, quotient first. A zero divisor yields two zeroes.

### Adding an entry

1. Write the reference implementation in Lap.
2. Add the entry and its lowering.

## Backend

`lapc` emits C17 and compiles it with a C compiler. The runtime is C, compiled and linked in the same step. An intrinsic's lowering is a C expression.

A value of 64 bits or fewer is a `uint64_t`. A wider value is a `uint64_t` array of `ceil(width / 64) + 1` words; the extra word is slack for a bit-granular store that straddles a word boundary. A reference is a base pointer and a bit offset, passed as two arguments. `NAND` is `~` of `&`, masked to the width.

The backend emits straightforward C and leaves optimization to the C compiler. It does not promote a binding to a register, inline a callee, or strength-reduce a multiply or a division; the C compiler does those. A self-tail-call becomes a `goto` to the body when no argument is a reference to a local of that frame. Only functions reachable from `main` are emitted; a program without `main` emits every function.

A wide operation is a loop over words. It is defined for `not`, `and`, `or`, `xor`, `add`, `sub`, and `nand`. A wide value is loaded and stored a word at a time, at any bit offset. A division is defined for operands up to 64 bits; a wider operand is a codegen error.

## Runtime interface

The runtime provides the operations in [EXTERN.md](EXTERN.md) and the program's entry and exit. It is C: it calls the exported `lap_main` and provides one entry point per operation, named `lap_extern_` followed by the table label with dots replaced by underscores, such as `lap_extern_memory_acquire`. An entry point takes the operation's arguments followed by an out buffer, so a lowered call reaches its operation directly. `lap_extern` dispatches by value for callers that need it: it takes the operation, up to four words, and an out buffer.

- `main` is the entry point. It takes no parameters and returns a determinate `BIT`: `BIT.ZERO` exits with status 0, `BIT.ONE` with status 1. `process.exit` overrides it.
- The stack grows downward. A self-tail-call is a jump to the body, so it does not grow the stack. It is a jump only when no argument is a reference to a local of that frame; a reference to a reference parameter is passed through.
- The runtime owns the handle table, the stream buffers, the clocks, and the entropy source. [EXTERN.md](EXTERN.md) owns their contracts.

## Limits

`lapc` imposes no limit on the size of a type, a value, or a program beyond memory. The stack bounds a value wider than 64 bits and non-tail recursion.

## Diagnostics

- A determinacy error must name the source: a bare `BIT`, an unwritten slot, or an unchecked `EXTERN` payload.
- An `EXTERN` error must name the cause: an unknown operation, a wrong argument count, a width mismatch, or a result that does not match its context.

A store through a reference must be determinate, because the target may be the caller's slot. A call with a reference argument leaves the target determinate but unknown.
