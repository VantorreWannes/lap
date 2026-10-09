# lapc-tokenize

The command-line interface for `lapc-tokenizer`.

## Scope

This crate is a binary. It contains no lexical logic of its own; all tokenizing behavior lives in `lapc-tokenizer`.

## Usage

```
lapc-tokenize <file> [--format <format>]
```

The program reads exactly one file. It does not resolve imports and does not read any other file.

## Output

On success, the program writes the token list to standard output in the requested format:

- `json`: compact JSON (the default).
- `pretty`: indented JSON.
- `text`: one token per line, as `<start>..<end> <kind>`.

On failure, the program writes the lexical error to standard error and exits with a non-zero status.
