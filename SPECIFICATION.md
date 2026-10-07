# Labels and Primitives Language Specification

## 1. Syntax

### 1.1 Grammar

```text
Program       := Binding*
Binding       := Target "=" Expr

Target        := "*"? Name | "[" Target ("," Target)* "]"

Func          := "(" ParamList? ")" ReturnType "{" Body "}"
ParamList     := Param ("," Param)*
Param         := Label ":" "*"? TypeName
TypeName      := Name | "BIT"
ReturnType    := TypeName

Body          := Stmt*
Stmt          := Binding | Expr

Expr          := "*"? Name
               | Func
               | Call
               | "[" (Expr ("," Expr)*)? "]"
               | "BRANCH" "(" Expr ")" "{" Body "}" "{" Body "}"

Call          := Name "(" (Expr ("," Expr)*)? ")"

Label         := [A-Za-z_][A-Za-z0-9_]*
Name          := Label ("." Label)*
```

### 1.2 Lexical Conventions

- **Labels (`Label`):** The atomic identifier unit. Must start with an ASCII letter or underscore (`[A-Za-z_]`), followed by any sequence of letters, digits, or underscores (`[A-Za-z0-9_]*`). A `Label` cannot contain periods.
- **The Period Token (`.`):** The period is a delimiter token that **only exists strictly between two labels** (`Label ("." Label)*`).
  - Leading dots (`.foo`), trailing dots (`foo.`), and consecutive dots (`foo..bar`) are syntax errors.
  - Chaining is valid (e.g., `STD.u8.add`).
- **Scoping Semantics:** Dot paths express hierarchy and containment (e.g., `STD.u8.add` refers to `add` inside `u8` inside `STD`). There are no access modifiers (no `public`, `private`), hidden receiver arguments (`this`/`self`), or method call sugar; any valid path can be called or referenced directly by any caller.
- **Comments:** `//` begins a single-line comment that extends to the end of the line.
- **Whitespace:** Space, tab, and newline characters separate tokens and carry no semantic weight.

---

## 2. Type System

### 2.1 Base Scalar and Array Shapes

There are only two forms of types in the language:

1. **`BIT`:** The primitive single-bit scalar value.
2. **Composite Arrays:** Fixed-length sequences defined as `[T_0, T_1, ..., T_N]`.

There are no implicit types. All multi-bit abstractions are type aliases constructed from arrays of `BIT`:

```text
u2 = [BIT, BIT]
u4 = [u2, u2]
```

### 2.2 Return Signatures

Function declarations require an explicit return type name (`BIT` or a previously defined alias). Inlining literal composite shapes in a return signature is invalid:

```text
// Invalid:
make_pair = () [BIT, BIT] { [BIT.zero, BIT.one] }

// Valid:
u2 = [BIT, BIT]
make_pair = () u2 { [BIT.zero, BIT.one] }
```

### 2.3 Structural Equivalency

Type compatibility is determined purely by bit-width and composite layout. If two types resolve to the exact same sequence of bits, they are interchangeable.

### 2.4 Absence of Literals and Base Constants

The language contains no literal numbers or scalar literals. The two physical states of a bit are provided by the `STD` primitive as `BIT.zero` (ground / low) and `BIT.one` (VCC / high). Multi-bit values are constructed by packaging bit constants into arrays:

```text
zero = BIT.zero
one = BIT.one
nibble = [zero, one, zero, one]
```

---

## 3. Core Primitives

The language includes exactly six reserved primitive symbols:

| Primitive | Classification      | Description                                                |
| :-------- | :------------------ | :--------------------------------------------------------- |
| `BIT`     | Type                | The base scalar bit.                                       |
| `NAND`    | Gate                | Universal logic function: `NAND(x: BIT, y: BIT) BIT`.      |
| `BRANCH`  | Control Flow        | Primitive single-bit conditional branching construct.      |
| `SYS`     | System Gate         | Host kernel and platform syscall interface.                |
| `STD`     | Library / Intrinsic | Root namespace for baseline types and compiler intrinsics. |
| `*`       | Reference Operator  | Marker for downward in-place mutation aliases.             |

### 3.1 `NAND`

`NAND` is the fundamental operational primitive of the language. It operates exclusively on two `BIT` values and produces a `BIT` according to standard hardware truth tables:

$$NAND(0, 0) = 1, NAND(0, 1) = 1, NAND(1, 0) = 1, NAND(1, 1) = 0$$

where $0$ represents `BIT.zero` and $1$ represents `BIT.one`.

### 3.2 `BRANCH`

Conditional control flow is evaluated via `BRANCH`:

```text
result = BRANCH(condition) {
    // Evaluated when condition == BIT.one
    branch_a_value
} {
    // Evaluated when condition == BIT.zero
    branch_b_value
}
```

- `condition` must resolve to a single `BIT`.
- Both branch bodies must yield values of identical structural type.
- The `BRANCH` construct is an expression; it evaluates to the value of the executed branch block's final expression.

---

## 4. The `STD` Primitive & Compiler Hoisting

`STD` is the built-in root namespace containing standard hardware abstractions, data types, foundational constants, and arithmetic logic.

### 4.1 Canonical Constants and Types

`STD` provides the primitive scalar values:

- `BIT.zero`: Low / ground bit state.
- `BIT.one`: High / VCC bit state.

`STD` exports uniform binary types:

- `STD.u8` = `[BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]`
- `BIT6` = `[STD.u8, STD.u8]`
- `STD.u32` = `[BIT6, BIT6]`
- `STD.u64` = `[STD.u32, STD.u32]`

### 4.2 Gate Definitions and Native Hoisting

Every operational label in `STD` has a canonical software definition implemented entirely through `NAND` gates and `BRANCH` statements:

```text
// Logical NOT defined inside STD
BIT.not = (x: BIT) BIT {
    NAND(x, x)
}
```

Compilers targeting native instruction sets recognize canonical `STD` identifiers (such as `STD.u8.add`, `STD.u32.add`, `STD.u64.mul`) and substitute their gate networks with equivalent machine instructions.

When compiling for microcontrollers or verification simulators lacking hardware ALU units, the compiler emits the raw fallback gate networks without modification.

---

## 5. System Calls (`SYS`)

The `SYS` primitive interfaces with the host platform's execution environment. It accepts an operation identifier followed by positional arguments.

```text
SYS(op_code, arg1, arg2, ...)
```

Arguments are passed either directly by value (loaded into registers) or by reference (`*`) to provide pointers to contiguous memory buffers.

### 5.1 Supported Operations

| Operation | Signature | Semantics |
| :--- | :---------------------------------- | :------------------------------------------------------------------------------------------------ |
| `SYS.read` | `SYS(SYS.read, fd, *buf, count)` | Reads up to `count` bytes from descriptor `fd` into the memory backing `buf`. Returns bytes read. |
| `SYS.write` | `SYS(SYS.write, fd, *buf, count)` | Writes `count` bytes from `buf` to descriptor `fd`. Returns bytes written. |
| `SYS.open` | `SYS(SYS.open, *path, flags, mode)` | Opens the resource at null-terminated path `path`. Returns a file descriptor. |
| `SYS.close` | `SYS(SYS.close, fd)` | Releases the open file descriptor `fd`. Returns `BIT.zero` on success. |
| `SYS.exit` | `SYS(SYS.exit, code)` | Terminates execution of the current process with status `code`. Does not return. |

<details>
<summary>Syscall ABI and Platform Conventions</summary>

- On Linux x86_64: `op_code` is loaded into `rax`. Arguments map sequentially to `rdi`, `rsi`, `rdx`, `r10`, `r8`, and `r9`.
- When passing an argument prefixed with `*` (such as `*buf`), the memory address of the first bit of the variable on the call stack is forwarded to the host kernel.
- The return value of `SYS` resolves to an array matching the host register size (`STD.u64` on 64-bit systems).

</details>

---

## 6. Storage Model & References

### 6.1 Value Semantics

Every assignment (`a = b`) performs a full bit-copy of `b` into the memory slot reserved for `a`.

### 6.2 Reference Mutation (`*`)

References provide in-place mutation without pointer arithmetic or dynamic allocation:

- **Reference Parameters:** Marking a function argument with `*` (`p: *Type`) passes the memory address of the caller's variable rather than copying its contents.
- **Transparent Assignment:** Reassigning a reference variable modifies the underlying caller storage:
```text
    clear_bit = (target: *BIT) BIT {
        target = BIT.zero
        BIT.zero
    }
```
- **Reference Unbinding:** Arrays can be destructured into references bound to internal elements:
```text
    [*first, *second] = pair
    first = BIT.one // Modifies the first slot of 'pair' directly
```

### 6.3 Downward-Only Lifetimes

References cannot escape upward:

1. Functions cannot declare reference return types (`*Type` is prohibited in `ReturnType`).
2. References cannot be assigned to non-reference bindings or stored inside value arrays.

Because references only travel downward with call frames, stack slots are never accessed after their owning frame is popped.

---

## 7. Execution and Control Flow

1. **Evaluation Order:** Statements in a body execute strictly sequentially.
2. **Implicit Block Returns:** The final statement of any `Func` body or `BRANCH` block must be an expression, which serves as the block's return value.
3. **Iteration:** There are no loop keywords. All repetition is achieved via recursion. Compilers guarantee tail-call optimization when the recursive call is the terminal expression in a code path.