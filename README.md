# Labels and Primitives

Labels and Primitives (Lap) is a cross-paradigm programming language built from eight primitives.

## Scope

Lap answers one question: what is the smallest set of primitives a whole language can emerge from?

Lap provides no numbers, no arithmetic, no strings, and no standard library. Those are library code written in Lap. See [Emergence](#emergence).

## Primitives

| Primitive                         | Role                                                 |
| --------------------------------- | ---------------------------------------------------- |
| `BIT`                             | the indeterminate value                              |
| `NAND`                            | the only operation                                   |
| `BRANCH`                          | selection between two blocks                         |
| `[item, ...]`                     | ordered, heterogeneous grouping                      |
| `(param: Type, ...) Type { ... }` | function definition                                  |
| `label = ...`                     | binding of a label to a value, a function, or a type |
| `*label`                          | alias of the slot `label` resolves to                |
| `EXTERN`                          | the only exit from the language                      |

## Rules

### Determinacy

Every occurrence of `BIT` denotes the same indeterminate value. An expression is determinate when its value is the same under both states of that value. A program may only depend on determinate expressions.

`NAND(BIT, NAND(BIT, BIT))` is determinate and is one. A bare `BIT` is not determinate. An unwritten slot holds an indeterminate value, and two such slots are unrelated.

### Types

Every label is statically typed. A type is a tree of `BIT`s and collections, and two types with the same tree are the same type. A reference type `*Type` is the type of a slot of that type.

A binding whose right-hand side is a type expression binds a type, and the label is an alias for that type. A type expression is `BIT`, a label bound to a type, `*Type`, or a collection of types.

### Block value

A block evaluates to its terminal bare expression, or to `[]` when it ends in a binding.

### Names

A name is global or local. A global name is defined once; shadowing it is an error. A parameter or block binding is local to its function or block.

A name is visible from its binding onward. A function's name is also visible in its own body, so a function may call itself.

### References

A reference is an alias, not a value. Reading it reads the aliased slot; assigning to it writes the aliased slot. A `*Type` parameter aliases the caller's slot, so `*param` forwards that same slot.

A reference may alias only downward or sideways on the stack. A function therefore cannot return a reference, and a reference cannot be stored in a collection. Dangling is unrepresentable rather than checked.

A reference may be a destructuring target.

### External memory

`EXTERN` is an optimization barrier. Heap storage is addressed by value handles, never by `*` references.

## Emergence

Constants, logic, arithmetic, numeric types, and control structures are library code. The language gains nothing when a library gains a capability.

Library code reaches native speed because a compiler recognizes it, not because the language grows. See [COMPILER.md](COMPILER.md).

The logic operators derive from `NAND`:

- `not(a)` is `NAND(a, a)`
- `and(a, b)` is `not(NAND(a, b))`
- `or(a, b)` is `NAND(not(a), not(b))`
- `xor(a, b)` is `NAND(NAND(a, n), NAND(b, n))` where `n` is `NAND(a, b)`

## Example

```
BIT.ONE = NAND(BIT, NAND(BIT, BIT))
BIT.ZERO = NAND(BIT.ONE, BIT.ONE)

U2 = [BIT, BIT]
PAIR = [BIT, BIT]
TUPLE = [BIT, U2, PAIR]

// PAIR and U2 are the same type, so no conversion is needed.

bit.not = (a: BIT) BIT {
    NAND(a, a)
}

// A reference parameter writes through to the caller's slot.
bit.toggle = (target: *BIT) [] {
    target = bit.not(target)
}

// A reference parameter forwards as a reference.
bit.toggle.twice = (target: *BIT) [] {
    bit.toggle(*target)
    bit.toggle(*target)
}

// Both blocks yield U2. The bare expression is last.
u2.select = (flag: BIT, when.one: U2, when.zero: U2) U2 {
    BRANCH (flag) {
        when.one
    } {
        unused: BIT = BIT.ZERO
        when.zero
    }
}

// The heap is reached by handle, not by reference.
// U64 and the operation constants are elided.
heap.alloc = () [BIT, U64] {
    EXTERN(OP.MEMORY.ACQUIRE)
}

heap.free = (handle: *U64) [] {
    [released] = EXTERN(OP.MEMORY.RELEASE, handle)
    handle = U64.ZERO
}

main = () BIT {
    state: BIT = BIT.ZERO

    alias: *BIT = *state
    alias = BIT.ONE

    bit.toggle.twice(*state)

    low: U2 = [BIT.ZERO, BIT.ONE]
    high: U2 = [BIT.ONE, BIT.ONE]
    chosen: U2 = u2.select(state, low, high)
    paired: PAIR = chosen

    bundle: TUPLE = [state, chosen, paired]
    [head, *middle, last] = bundle

    // The handle is determinate only in the success block.
    [acquired, handle] = heap.alloc()
    BRANCH (acquired) {
        heap.free(*handle)
    } {
        []
    }

    head
}
```

## Grammar

Whitespace and comments are stripped before parsing. A label is a keyword only when it equals that keyword exactly, so `BIT.ONE` is a label and `BIT` is not.

```ebnf
Program        ::= Statement*

Statement      ::= Binding | Expression

Binding        ::= Target "=" (Expression | FunctionDef)
Target         ::= TypedIdent
                 | "*" Label
                 | "[" Target ("," Target)* "]"

TypedIdent     ::= Label (":" Type)?

Expression     ::= Primary
                 | ExternExpr
                 | BranchExpr
                 | NandExpr
                 | CollectionExpr
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
Keyword        ::= "BIT" | "NAND" | "BRANCH" | "EXTERN"
Comment        ::= "//" [^\n]*
```

## Ownership

- This document owns the language: the primitives, the rules, and the grammar.
- A compiler owns its lowering and its runtime interface. [COMPILER.md](COMPILER.md) records those for `lapc`.
- A library owns everything else.

A compiler may not add a primitive, a type rule, or a syntax form.

## Stability

The primitives, the rules, and the grammar are frozen. A program that is valid today stays valid, and a compiler that rejects it is wrong.

New capability arrives as a library, or as compiler recognition of a library, never as a language change.
