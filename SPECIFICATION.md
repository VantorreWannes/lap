# Lables and Primitives

## 1. Syntax

### 1.1 EBNF Grammar

```text
Program       := Binding*
Binding       := Target "=" Expr

Target        := "*"? Name | "[" Target ("," Target)* "]"

Func          := "(" ParamList? ")" Name "{" Body "}"
ParamList     := Param ("," Param)*
Param         := Name ":" "*"? Name

Body          := Stmt*
Stmt          := Binding | Expr

Expr          := Literal 
               | "*"? Name 
               | Func
               | Call 
               | "[" (Expr ("," Expr)*)? "]"
               | "BRANCH" "(" Expr ")" "{" Body "}" "{" Body "}"

Call          := Name "(" (Expr ("," Expr)*)? ")"

Literal       := [0-9]+ | "0x" [0-9a-fA-F]+
Name          := [A-Za-z_.][A-Za-z0-9_.]*
```

### 1.2 Lexical Rules
- Identifiers match `Name`.
- Whitespace is non-semantic outside of token separation.
- `//` comments extend to the end of the line.

---

## 2. Type System

### 2.1 Types
1. **`BIT`:** The base scalar value (`0` or `1`).
2. **Arrays:** Fixed sequences of shapes `[T_0, T_1, ..., T_N]`.

There are no native primitive numerical types (`u8`, `i32`, `usize`). All higher-level types are userland aliases of nested arrays of `BIT`.

### 2.2 Equivalency and Sizing
- Types are strictly structurally equivalent.
- Shapes must match bit-for-bit during assignment, unbinding, and argument passing.
- Any shape or width mismatch is an immediate compile-time error (**loud crash**).

### 2.3 Literals
- `0` and `1` represent single `BIT` values.
- Integer literals greater than 1 zero-extend upward to fit the exact bit-width demanded by the receiving context. Contexts with unspecified width cause a compile-time failure.

---

## 3. Primitives & Intrinsics

The runtime defines exactly five built-in symbols:

### 3.1 `BIT`
The primitive scalar type identifier.

### 3.2 `NAND(x, y)`
- Arguments: `x: BIT`, `y: BIT`
- Returns: `BIT`
- Semantics: Hardware NAND truth table.

### 3.3 `BRANCH(cond) { TrueBody } { FalseBody }`
- Condition: `cond: BIT`
- Semantics: If `cond == 1`, evaluates `TrueBody`; else evaluates `FalseBody`.
- Type Rule: Both `TrueBody` and `FalseBody` must resolve to the identical shape.
- Value: Evaluates directly to the final expression of the executed body.

### 3.4 `SYS(op, ...)`
- Low-level gate to host kernel / hypervisor syscalls.
- Binds directly to platform calling conventions. Arguments may be passed by value or via `*` reference to back memory buffers.

### 3.5 `*`
The reference operator used in parameters and bindings.

---

## 4. Execution Model & Storage

### 4.1 Value Model
- Variables are pure value slots by default.
- Assignment (`a = b`) performs a direct bit-copy from `b` into `a`.

### 4.2 Reference Semantics (`*`)
- References are zero-cost aliases to existing backing storage.
- **Passing by reference:** `param: *Type` passes the caller's storage location without bit-copying.
- **Transparent write-through:** If `p` is an alias (created via `*p_ref` or `*param`), evaluating `p = val` modifies the aliased backing store directly.
- **No re-pointing:** An established reference cannot be changed to refer to different storage.

### 4.3 Reference Unbinding
Arrays can be unpicked into references:
```text
[*lo, hi] = pair
```
`lo` becomes a transparent alias to the first element slot of `pair`. Modifying `lo` writes into that slot in `pair`.

### 4.4 Downward-Only Lifetimes
- Functions **cannot** return references (`*Type` is invalid as a return type).
- References cannot be written into non-reference arrays or stored across function boundaries.
- Because references only propagate down the call stack, dangling pointers and use-after-free are structurally impossible.

---

## 5. Control Flow & Evaluation

1. Evaluation order within bodies is strictly sequential (top to bottom).
2. The last statement in a `Func` or `BRANCH` block must be an `Expr`. This expression is the block's resolved value.
3. Recursion is allowed. Loops are expressed purely via recursion.
4. There are no runtime loops (`for`, `while`) or break statements.

---

## 6. Complete Program Example

```text
// ==========================================
// 1. Primitive Logic & Gate Derivations
// ==========================================
ON  = (x: BIT) BIT { NAND(x, NAND(x, x)) }
OFF = (x: BIT) BIT { NAND(ON(x), ON(x)) }

u1.not = (x: BIT) BIT { NAND(x, x) }
u1.and = (a: BIT, b: BIT) BIT { NAND(NAND(a, b), NAND(a, b)) }
u1.xor = (a: BIT, b: BIT) BIT {
    ab = NAND(a, b)
    NAND(NAND(a, ab), NAND(b, ab))
}

// ==========================================
// 2. Type Shapes
// ==========================================
u2 = [BIT, BIT]
u4 = [u2, u2]

// ==========================================
// 3. Userland Half-Adder
// ==========================================
half_adder = (a: BIT, b: BIT) [BIT, BIT] {
    sum  = u1.xor(a, b)
    cout = u1.and(a, b)
    [sum, cout]
}

// ==========================================
// 4. In-Place 4-Bit Incrementer
// ==========================================
u4.inc = (val: *u4) BIT {
    [[*b0, *b1], [*b2, *b3]] = val

    [s0, c0] = half_adder(b0, ON(0))
    [s1, c1] = half_adder(b1, c0)
    [s2, c2] = half_adder(b2, c1)
    [s3, c3] = half_adder(b3, c2)

    b0 = s0
    b1 = s1
    b2 = s2
    b3 = s3

    c3 // returns overflow bit
}

// ==========================================
// 5. Entry Point
// ==========================================
main = () BIT {
    z = OFF(0)
    // 4-bit integer literal: 0b0011 (3)
    counter = [[ON(0), ON(0)], [z, z]]

    // Increment in place -> transforms to 0b0100 (4)
    overflow = u4.inc(counter)

    // Verify bit 2 is set high
    [[r0, r1], [r2, r3]] = counter
    r2
}
```
