# lapc-tokenizer

A library that converts Lap source text into typed tokens for the parser.

## Scope

The tokenizer answers one question:

> Which lexical items are present, and where are they located?

It does not build expressions, validate grammar structure, or provide a command-line interface.

## Public API

```rust
pub fn lex(source: &str) -> Result<Vec<Token>, LexError>;
```

The public types are the boundary between this crate, later compiler stages, and external tools:

- `TokenKind` is a sum type. Each variant is one lexical item. A variant that carries a value stores it directly, so the label variant holds its name. Variants that carry no value, such as keywords and punctuation, hold nothing.
- `Span` is a byte range in the source.
- `Token` is the root struct. It pairs a `TokenKind` with the `Span` that locates it.
- `LexError` identifies an invalid lexical item and its location.

```rust
pub struct Token {
    kind: TokenKind,
    span: Span,
}

pub enum TokenKind {
    Label(String),   // carries the label name
    Bit,             // keywords and punctuation carry no value
    // ...
}
```

Fields are private. Read them through accessors, such as `Token::kind` and `Token::span`.

A token carries both a kind and a span. The kind says what the token is and holds its value; the span says where it is. Location and content are independent, so neither replaces the other. A label token needs its name and its position; a punctuation token needs only its position.

All public data types derive `serde::Serialize` and `serde::Deserialize`. This allows stages and external tools to exchange tokenizer data without depending on internal implementation details.

A successful result contains only valid tokens. An error contains its location and no partial token stream.

## Ownership

This crate is the single source of truth for:

- the lexical token set;
- source spans;
- label validation;
- lexical errors.

The label rule is:

```text
[a-zA-Z0-9]+([_.][a-zA-Z0-9]+)*
```

The tokenizer validates this rule once. Later stages consume the result and do not repeat the validation.

The parser owns expression structure and grammar rules. A separate binary crate owns command-line behavior.

## Compatibility

Serialized public types are part of the crate's public API. Changes to them can affect parsers, tools, and community extensions. Treat those changes as API changes.
