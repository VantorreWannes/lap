# lapc

`lapc` is a compiler for Lap. This document records the choices it makes that the language does not.

## Scope

[README.md](README.md) owns the language. This document owns the lowering, the intrinsic table, the backend, and the runtime interface. [EXTERN.md](EXTERN.md) owns the external operation table.

`lapc` adds no syntax and no type rule. A program it accepts is accepted by any conforming compiler.

## Input

`lapc` takes one or more files and appends them in the order given, top to bottom. The result is one program.

## Operation erasure

Every operation in Lap is a `NAND` tree, so arithmetic written as a library is slow unless the compiler recognizes it. Erasure is that recognition: a function whose label is in the table is replaced by a native operation.

This is why the language does not need arithmetic primitives.

### Table

Each entry names an intrinsic and its lowering. The label is the contract: a function with this label must behave as the intrinsic.

```
operation u8.add:
    lowering: iadd
```

The table is hardcoded. The compiler does not discover entries.

### Matching

1. Look up the function's label in the table.
2. On a hit, replace the function with the intrinsic node.

An intrinsic is a first-class IR node, not an `EXTERN` call, so later passes still see it. Erasure never introduces an `EXTERN` call, and never removes one.

### Contracts

- Matching is by label.
- Only a whole function matches. Erasure runs before inlining.
- A miss is never an error. The `NAND` tree is correct, only slower.

### Adding an entry

1. Write the reference implementation in Lap.
2. Add the entry and its lowering.

## Backend

`lapc` lowers to Cranelift IR and emits an object file. The runtime is C, linked by the system linker. An intrinsic's lowering is a Cranelift IR sequence.

## Runtime interface

The runtime provides the operations in [EXTERN.md](EXTERN.md) and the program's entry and exit. It is C: it calls the exported `lap_main` and provides `lap_extern`.

- `main` is the entry point. It takes no parameters and returns a determinate `BIT`: `BIT.ZERO` exits with status 0, `BIT.ONE` with status 1. `process.exit` overrides it.
- The stack grows downward. A tail call may reuse the caller's frame only when no reference to a local of that frame is passed.
- The runtime owns the handle table, the stream buffers, the clocks, and the entropy source. [EXTERN.md](EXTERN.md) owns their contracts.

## Diagnostics

- A determinacy error must name the source: a bare `BIT`, an unwritten slot, or an unchecked `EXTERN` payload.
- An `EXTERN` error must name the cause: an unknown operation, a wrong argument count, a width mismatch, or a result that does not match its context.
