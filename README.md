# Labels and Primitives

Labels and Primitives (Lap) is a timeless, cross-paradigm, unchangeable programming language. Only the tooling and compiler implementations around it may change.

This document is the language definition. Compiler crates implement it; they do not extend or reinterpret it.

## Primitives

The language is built from eight primitives. Every other construct derives from them.

- `BIT`: The data primitive. Initialized to an arbitrary state.
- `NAND`: The operation primitive. Computes Sheffer stroke over two `BIT` values.
- `BRANCH`: The branching primitive. Selects an expression block based on a `BIT`.
- `[item, ...]`: The collection primitive. Groups ordered, heterogeneous elements.
- `(param: Type, ...) Type { ... }`: The function definition primitive.
- `label = ...`: The assignment primitive. Copies right-to-left into storage.
- `*label`: The reference primitive. Aliases an existing storage slot.
- `EXTERN`: The external primitive. Accesses syscalls, allocators, and intrinsics.

## Core Rules

1. **Bare Storage:** Every value is either an explicit primitive or held in a labeled memory slot. Labels are statically typed and invariant. Types are nominal: explicit casting functions must be implemented to convert between different types, even if they share the same layout.
2. **Infix Label Grammar:** Labels allow `_` and `.` strictly between alphanumeric characters: `[a-zA-Z0-9]+([_.][a-zA-Z0-9]+)*`. Neither `_` nor `.` may begin or end a label.
3. **Rust-Like Expression Blocks:** The terminal expression in a block produces its return value. A statement terminating with an assignment yields no value and cannot be returned.
4. **Reference Lifetimes:** Functions cannot return reference types (`*Type`). References only alias downward or sideways on the stack, preventing dangling references by design.
5. **External Memory:** `EXTERN` acts as an optimization barrier. Dynamic heap storage is addressed using value handles (e.g., bit collections), never through language-level `*` references.

## Canonical Example

The following annotated program demonstrates bootstrapping, nominal typing, explicit casting, pass-by-reference mutation, heterogeneous collections, unbinding, expression-oriented branching, and heap interactions via `EXTERN`.

```
// 1. BOOTSTRAPPING CONSTANTS
// Bits begin in an arbitrary state. Standard values must be algebraically derived.
seed = BIT
ONE: BIT = NAND(seed, NAND(seed, seed))
ZERO: BIT = NAND(ONE, ONE)

// 2. NOMINAL TYPES AND EXPLICIT CASTING
// Types are invariant. Identical underlying layouts require explicit casts.
U2 = [BIT, BIT]
PAIR = [BIT, BIT]

cast.u2.to.pair = (src: U2) PAIR {
    [b0, b1] = src
    [b0, b1]
}

// Heterogeneous collection containing different types
TUPLE = [BIT, U2, PAIR]

// 3. LOGIC OPERATORS
not = (a: BIT) BIT {
    NAND(a, a)
}

xor = (a: BIT, b: BIT) BIT {
    n = NAND(a, b)
    NAND(NAND(a, n), NAND(b, n))
}

// 4. REFERENCES AND IN-PLACE MUTATION
// Functions cannot return references, but parameters can be passed as references.
toggle = (target: *BIT) [] {
    target = not(target)
    []
}

// 5. BRANCHING AND BLOCK EVALUATION
// Both branches must yield the same type. The final bare expression is returned.
choose = (flag: BIT, opt.a: U2, opt.b: U2) U2 {
    BRANCH (flag) {
        opt.a
    } {
        // Assignments cannot serve as return values.
        // The bare expression opt.b is placed last.
        dummy: BIT = ZERO
        opt.b
    }
}

// 6. HEAP MANAGEMENT VIA EXTERN
// Heap pointers are stored as data handles, not language references.
OP.ALLOC = [ZERO, ONE]
OP.FREE  = [ONE, ZERO]

alloc.slot = () U2 {
    EXTERN(OP.ALLOC)
}

free.slot = (handle: *U2) [] {
    EXTERN(OP.FREE, handle)
    handle = [ZERO, ZERO]
    []
}

// 7. CANONICAL EXECUTION FLOW
main = () BIT {
    // Label storage declaration
    state: BIT = ZERO

    // Reference creation and write-through mutation
    alias: *BIT = *state
    alias = ONE
    // Both 'state' and 'alias' now store ONE

    toggle(*state)
    // 'state' is toggled back to ZERO

    // Instantiating collections
    first.u2: U2 = [ZERO, ONE]
    second.u2: U2 = [ONE, ONE]

    // Branch expression
    selected: U2 = choose(state, first.u2, second.u2)

    // Explicit casting between nominal types
    paired: PAIR = cast.u2.to.pair(selected)

    // Heterogeneous collection grouping and unbinding
    bundle: TUPLE = [state, selected, paired]
    [head.bit, *tail.u2, out.pair] = bundle

    // Heap allocation pattern
    heap.ptr: U2 = alloc.slot()
    free.slot(*heap.ptr)

    // Return the final evaluated bit
    head.bit
}
```

## Truth Values

The standard logic operators are derived from `NAND`:

- `not a = NAND(a, a)`
- `a and b = NAND(NAND(a, b), NAND(a, b))`
- `a or b = NAND(NAND(a, a), NAND(b, b))`
- `a xor b = NAND(NAND(a, NAND(a, b)), NAND(b, NAND(a, b)))`

## Formal Grammar

Whitespace and comments are stripped before parsing.

```ebnf
Program        ::= Statement*

Statement      ::= Binding | Expression

Binding        ::= Target "=" Expression
Target         ::= TypedIdent
                 | "*" Label
                 | "[" Target ("," Target)* "]"

TypedIdent     ::= Label (":" Type)?

Expression     ::= Primary
                 | ExternExpr
                 | BranchExpr
                 | NandExpr
                 | CollectionExpr
                 | FunctionDef
                 | CallExpr

Primary        ::= Label | "*" Label | "BIT"

NandExpr       ::= "NAND" "(" Expression "," Expression ")"
BranchExpr     ::= "BRANCH" "(" Expression ")" Block Block
ExternExpr     ::= "EXTERN" "(" Expression ("," Expression)* ")"
CollectionExpr ::= "[" (Expression ("," Expression)*)? "]"
CallExpr       ::= Label "(" (Expression ("," Expression)*)? ")"

FunctionDef    ::= "(" ParamList? ")" Type Block
ParamList      ::= Param ("," Param)*
Param          ::= Label ":" Type
Type           ::= "BIT"
                 | Label
                 | "*" Type
                 | "[" (Type ("," Type)*)? "]"

Block          ::= "{" Statement* Expression? "}"
Label          ::= [a-zA-Z0-9]+ ( ("_" | ".") [a-zA-Z0-9]+ )*
Comment        ::= "//" [^\n]*
```
