# Compiler Implementation Choices

This document details the concrete operational decisions and runtime interfaces implemented by this compiler (`lapc`).

## 1. Multi-File Compilation and Ordering

- Source files passed via the command line interface are parsed independently.
- Statements at the root level of each file are appended into a single translation unit in the exact order they appear in the CLI arguments:
  `lapc file_a.lap file_b.lap file_c.lap`
  Statements from `file_a.lap` are placed first, followed by `file_b.lap`, and then `file_c.lap`.
- All root-level labels are globally visible across all source files.
- Local labels (such as those inside functions and blocks) are strictly scoped to their declaring block and are inaccessible from outer scopes.
- Redefining an existing root-level label anywhere across the combined program causes a compile-time collision error.

## 2. Program Lifecycle and Entry Point

- Top-level assignments and statements execute sequentially in the order they were merged during program initialization.
- The program entry point must be a globally defined function named `main`:
  `main = () BIT { ... }`
- Execution order:
    1. All top-level bindings across all concatenated files are initialized.
    2. The compiler invokes `main()`.
    3. The `BIT` value returned by `main()` is used directly as the final exit status of the process.

## 3. Supported `EXTERN` Operations

The `EXTERN` primitive allows variable arguments, where the first argument is an operation code (`OP`), followed by any operation-specific parameters:
`EXTERN(OP, param_1, param_2, ...)`

### 3.1 Memory Management

Memory operations use collections of bits as opaque data handles to reference heap addresses:

- `EXTERN(OP.ALLOC, size)`
    - Parameter: `size` (collection of `BIT` specifying capacity).
    - Returns: An opaque handle collection pointing to the allocated storage.
- `EXTERN(OP.FREE, handle)`
    - Parameter: `handle` (collection returned by `OP.ALLOC`).
    - Returns: `[]`.
- `EXTERN(OP.READ, handle, offset)`
    - Parameters: `handle` (allocation handle), `offset` (bit offset collection).
    - Returns: `BIT`.
- `EXTERN(OP.WRITE, handle, offset, value)`
    - Parameters: `handle` (allocation handle), `offset` (bit offset collection), `value` (`BIT`).
    - Returns: `[]`.

### 3.2 Buffered Bit-Stream I/O

I/O does not assume ASCII or byte alignments. The runtime buffers single bits and only communicates with the external host when explicitly requested:

- `EXTERN(OP.PUSH_BIT, bit)`
    - Parameter: `bit` (`BIT`).
    - Returns: `[]`.
    - Appends a single bit to the runtime output buffer.
- `EXTERN(OP.FLUSH)`
    - Parameters: None.
    - Returns: `[]`.
    - Transmits all accumulated bits in the output buffer to the host environment.
- `EXTERN(OP.PULL_BIT)`
    - Parameters: None.
    - Returns: `BIT`.
    - Reads a single bit from the host input stream.

### 3.3 Process Termination

- `EXTERN(OP.EXIT, code)`
    - Parameter: `code` (`BIT`).
    - Returns: Does not return.
    - Halts execution immediately with the supplied exit bit.
