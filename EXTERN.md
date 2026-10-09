# lapc External Operations

This document owns the set of `EXTERN` operations `lapc` supports, and the contracts they carry.

## Scope

[README.md](README.md) owns `EXTERN` as a primitive. [COMPILER.md](COMPILER.md) owns lowering. This document owns the operation table.

`EXTERN` is the only exit from the language, so this table is the whole of what a Lap program can observe or affect.

An operation earns a place in the table when Lap cannot express it, or when a loop that expresses it moves one byte per call and the call is the cost the program pays. Everything else a program can build from the table belongs in a library.

## Call shape

An `EXTERN` call is `EXTERN(operation, argument, ...)`. The operation is a 16-bit value naming a table entry. Arguments and the result are values; a reference is never an argument, a payload, or a result.

### Encoding

A multi-bit value is an unsigned integer. Element 0 of a collection is the least significant bit. A nested collection flattens in order.

This is the one place `lapc` fixes a bit order. Numeric interpretation is otherwise a library concern.

### Width matching

The table declares each argument and each result field as a bit width. A type matches a width when its flattened layout is exactly that many bits.

Matching is by width, not by type, because the table declares widths. This is the only width rule in `lapc`, and it applies to nothing but `EXTERN`.

### Typing

The result type of an `EXTERN` call is the declared fields: a collection of the status bit followed by the payload fields, each a flat collection of its width. A context must match this type.

`lapc` rejects a call when the operation is not determinate, the operation is not in the table, the argument count is wrong, an argument width does not match, or the result does not match its context.

### Ordering

An `EXTERN` call is never reordered, duplicated, merged, or eliminated, including when its result is unused. Calls execute in program order.

## Results

Every operation returns a collection whose first field is a status bit. One is success, zero is failure. An operation with no payload returns a one-element collection.

On failure the payload is an indeterminate value. A program may only depend on determinate expressions, so a payload is usable only inside the success block of a `BRANCH` on the status bit. Checking is enforced by the determinacy rule rather than by a sum type.

Failure carries no detail. The language has no error taxonomy, and a status bit is the largest contract that needs no library support to read. A detailed variant of an operation can be added in a reserved group without changing anything here.

## Memory safety

A handle is an index into a runtime-owned table, not an address. The runtime owns every block, validates every handle, and bounds-checks every access.

An invalid handle, a released handle, or an out-of-range access is a failure, never a corrupt read or write. A Lap program cannot reach memory the runtime did not give it. This is why the heap is addressed by handle and `*` references stay on the stack.

## Granularity

A byte is the floor: the runtime's memory and streams are byte-addressed, and a bit-level operation would be a read, a mask, and a write, which is library code with no runtime meaning.

`stream.read.block` and `stream.write.block` move a byte range between a stream and a block. The per-byte loop that expresses the same transfer pays one call per byte, and the ordering rule forbids a compiler from merging those calls, so the table carries the bulk form and the byte stays the unit of every primitive. See [Buffering](#buffering).

## Buffering

A call is not a syscall. A memory operation reads or writes a runtime-owned block directly. A stream operation moves one byte between the program and a runtime-owned buffer, and only a full buffer, a flush, or a close reaches the operating system. A block operation moves its whole range through the same buffer.

A library loop that writes N bytes therefore costs N calls and N divided by buffer size syscalls. One `stream.write.block` costs one call.

Buffering is a property of the runtime, not a compiler transformation. Every call the program writes is still executed, in order. The ordering rule is unaffected.

Buffer size is unspecified and at least one byte. A program cannot observe it except through when bytes reach the operating system.

### Write durability

Success from `stream.write` or `stream.write.block` means the bytes are buffered. Only success from `stream.flush` means the buffered bytes reached the operating system.

A device failure can therefore surface at a later `stream.write` or at `stream.flush`, not at the call that supplied the byte. A program that needs to know a byte landed must flush.

On flush failure the buffer is discarded and the bytes are lost. Retrying a flush cannot resend them.

### Flush points

The runtime flushes a stream when its buffer fills, on `stream.flush`, on `stream.close`, and on `process.exit`.

A program that ends by returning from `main` is flushed. A program that ends any other way is not, and buffered bytes are lost.

Standard error is unbuffered. Each `stream.write` to it reaches the operating system, so a diagnostic survives an end that flushes nothing.

### Ordering between streams

Bytes written to one stream reach the operating system in program order.

Across two streams, relative order is guaranteed only at a flush. Interleaved writes to standard output and standard error appear in program order only if the program flushes standard output before each write to standard error.

### Reading

The runtime reads ahead into a buffer and `stream.read` takes one byte from it. A refill returns whatever is available and never waits for a full buffer, so an interactive stream stays responsive. `stream.read.block` drains the buffer first and then reads directly into the block, requesting no more than the block range holds.

Read-ahead consumes bytes from the operating system before the program asks for them. A stream handed to another program after a partial read is a non-goal.

`random.byte` is buffered the same way. It has no ordering requirement, so it needs no flush.

## Groups

The high byte of an operation is its group.

| Group    | Code             | Owns                               |
| -------- | ---------------- | ---------------------------------- |
| process  | `0x00`           | the program's own lifetime         |
| memory   | `0x01`           | runtime-owned blocks               |
| stream   | `0x02`           | byte sequences outside the program |
| clock    | `0x03`           | time                               |
| random   | `0x04`           | entropy                            |
| reserved | `0x80` to `0xFF` | never assigned by `lapc`           |

A group is a unit of extension. Adding an operation to a group, or a group to the table, changes no rule in this document.

## Table

Widths are in bits. Every result begins with the status bit, so only payload fields are listed.

| Operation         | Code     | Arguments                                   | Payload        |
| ----------------- | -------- | ------------------------------------------- | -------------- |
| `process.exit`    | `0x0000` | status 8                                    | none           |
| `memory.acquire`  | `0x0100` | length 64                                   | handle 64      |
| `memory.release`  | `0x0101` | handle 64                                   | none           |
| `memory.read`     | `0x0102` | handle 64, offset 64                        | byte 8         |
| `memory.write`    | `0x0103` | handle 64, offset 64, byte 8                | none           |
| `stream.open`     | `0x0200` | path block 64, offset 64, length 64, mode 8 | stream 64      |
| `stream.read`     | `0x0201` | stream 64                                   | byte 8         |
| `stream.write`    | `0x0202` | stream 64, byte 8                           | none           |
| `stream.flush`    | `0x0203` | stream 64                                   | none           |
| `stream.close`    | `0x0204` | stream 64                                   | none           |
| `stream.read.block` | `0x0205` | stream 64, block 64, offset 64, length 64 | count 64       |
| `stream.write.block` | `0x0206` | stream 64, block 64, offset 64, length 64 | none           |
| `clock.monotonic` | `0x0300` | none                                        | nanoseconds 64 |
| `clock.realtime`  | `0x0301` | none                                        | nanoseconds 64 |
| `random.byte`     | `0x0400` | none                                        | byte 8         |

`process.exit` does not return. It flushes every stream first. `lapc` still requires the enclosing block to be well-typed, and treats what follows as unreachable.

A block holds the length requested and no more. Its initial contents are indeterminate.

A path is a byte range in a block, so the table needs no string type. Mode 0 is read, 1 is write, 2 is append. Other modes are unassigned.

Streams 0, 1, 2, and 3 are open at entry. The first three are standard input, output, and error. Stream 3 is read-only and carries the program's arguments, each terminated by a zero byte.

`stream.read` fails at end of stream. Failure carries no detail, so end of stream and a device error are the same observation. `stream.read.block` fails only when no byte is available and `length` is not zero; a range it only partly fills succeeds with the count it took. `stream.write.block` buffers every byte of its range.

A block transfer fails on an invalid stream, an invalid block, or a range outside the block, and it leaves the block unchanged on failure.

`stream.flush` on a read-only stream succeeds and does nothing. `stream.close` flushes, and reports a flush failure as its own failure.

`clock.monotonic` never decreases across calls in program order. Its zero point is unspecified. `clock.realtime` is nanoseconds since the Unix epoch and may move in either direction.

## Derived in library

These have no table entry because a program can build them.

| Capability                | Built from                                   |
| ------------------------- | -------------------------------------------- |
| block length              | the length passed to `memory.acquire`        |
| block copy, fill, compare | `memory.read` and `memory.write`             |
| block resize              | `memory.acquire`, copy, `memory.release`     |
| buffered print            | `stream.write.block`, `stream.flush` once    |
| unbuffered write          | `stream.write` then `stream.flush`           |
| line buffering            | `stream.flush` after a newline byte          |
| argument count and text   | stream 3                                     |
| random fill, random range | `random.byte`                                |
| elapsed time              | two `clock.monotonic` calls                  |
| paths, strings, numbers   | blocks of bytes                              |

## Naming

The table is keyed by value. An operation label such as `OP.MEMORY.ACQUIRE` is library code, and `lapc` does not require or recognize any particular name.

## Example

```
// Library types and constants are elided.

memory.acquire = (length: U64) [BIT, HANDLE] {
    EXTERN(OP.MEMORY.ACQUIRE, length)
}

memory.release = (handle: HANDLE) [BIT] {
    EXTERN(OP.MEMORY.RELEASE, handle)
}

// The handle is determinate only inside the success block.
byte.roundtrip = (byte: U8) [BIT, U8] {
    [acquired, handle] = memory.acquire(U64.ONE)
    BRANCH (acquired) {
        [written] = EXTERN(OP.MEMORY.WRITE, handle, U64.ZERO, byte)
        [read, out] = EXTERN(OP.MEMORY.READ, handle, U64.ZERO)
        [released] = memory.release(handle)
        [read, out]
    } {
        [acquired, U8.ZERO]
    }
}
```

`[written]` and `[released]` are unused, and both calls still run.

A print is a loop of per-byte writes and one flush, so it costs one syscall.

```
print = (handle: HANDLE, length: U64) [BIT] {
    [sent] = stream.write.range(STREAM.OUT, handle, U64.ZERO, length)
    BRANCH (sent) {
        EXTERN(OP.STREAM.FLUSH, STREAM.OUT)
    } {
        [sent]
    }
}
```

## Non-goals

`lapc` does not provide failure detail, raw addresses, memory mapping, directory traversal, sockets, processes, or threads.

Each is a group or an entry away. None of them is a language change.
