use std::fmt;
use std::ops::Range;

use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct Span {
    start: usize,
    end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        assert!(start <= end);
        Self { start, end }
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn contains(&self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    pub fn join(self, other: Self) -> Self {
        Self::new(self.start.min(other.start), self.end.max(other.end))
    }
}

impl From<Span> for Range<usize> {
    fn from(span: Span) -> Self {
        span.start()..span.end()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TokenKind {
    Label(String),
    Bit,
    Nand,
    Branch,
    Extern,
    OpenBracket,
    CloseBracket,
    OpenParenthesis,
    CloseParenthesis,
    OpenBrace,
    CloseBrace,
    Comma,
    Colon,
    Star,
    Assign,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Label(name) => write!(f, "{name}"),
            Self::Bit => write!(f, "BIT"),
            Self::Nand => write!(f, "NAND"),
            Self::Branch => write!(f, "BRANCH"),
            Self::Extern => write!(f, "EXTERN"),
            Self::OpenBracket => write!(f, "["),
            Self::CloseBracket => write!(f, "]"),
            Self::OpenParenthesis => write!(f, "("),
            Self::CloseParenthesis => write!(f, ")"),
            Self::OpenBrace => write!(f, "{{"),
            Self::CloseBrace => write!(f, "}}"),
            Self::Comma => write!(f, ","),
            Self::Colon => write!(f, ":"),
            Self::Star => write!(f, "*"),
            Self::Assign => write!(f, "="),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Token {
    kind: TokenKind,
    span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn kind(&self) -> &TokenKind {
        &self.kind
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LexError {
    kind: LexErrorKind,
    span: Span,
}

impl LexError {
    pub fn new(kind: LexErrorKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn kind(&self) -> &LexErrorKind {
        &self.kind
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {}..{}",
            self.kind,
            self.span.start(),
            self.span.end()
        )
    }
}

impl std::error::Error for LexError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LexErrorKind {
    UnexpectedCharacter(char),
}

impl fmt::Display for LexErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedCharacter(character) => write!(f, "unexpected character {character:?}"),
        }
    }
}

pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    Lexer::new(source).tokenize()
}

struct Lexer<'a> {
    source: &'a str,
    position: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            position: 0,
        }
    }

    fn tokenize(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        while let Some(token) = self.next_token()? {
            tokens.push(token);
        }
        Ok(tokens)
    }

    fn next_token(&mut self) -> Result<Option<Token>, LexError> {
        self.skip_trivia();
        let start = self.position;
        let Some(character) = self.peek() else {
            return Ok(None);
        };

        if let Some(kind) = punctuation(character) {
            self.advance();
            return Ok(Some(Token::new(kind, Span::new(start, self.position))));
        }

        if character.is_ascii_alphanumeric() {
            self.scan_label();
            let name = &self.source[start..self.position];
            let kind = keyword(name).unwrap_or_else(|| TokenKind::Label(name.to_owned()));
            return Ok(Some(Token::new(kind, Span::new(start, self.position))));
        }

        Err(LexError::new(
            LexErrorKind::UnexpectedCharacter(character),
            Span::new(start, start + character.len_utf8()),
        ))
    }

    fn skip_trivia(&mut self) {
        loop {
            let Some(character) = self.peek() else {
                break;
            };
            if character.is_whitespace() {
                self.advance();
            } else if character == '/' && self.rest().starts_with("//") {
                self.skip_line_comment();
            } else {
                break;
            }
        }
    }

    fn skip_line_comment(&mut self) {
        match self.rest().find('\n') {
            Some(offset) => self.position += offset + 1,
            None => self.position = self.source.len(),
        }
    }

    fn scan_label(&mut self) {
        let source = self.source;
        let bytes = source.as_bytes();
        while self.position < bytes.len() && bytes[self.position].is_ascii_alphanumeric() {
            self.position += 1;
        }
        while self.position + 1 < bytes.len()
            && (bytes[self.position] == b'_' || bytes[self.position] == b'.')
            && bytes[self.position + 1].is_ascii_alphanumeric()
        {
            self.position += 1;
            while self.position < bytes.len() && bytes[self.position].is_ascii_alphanumeric() {
                self.position += 1;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn advance(&mut self) {
        if let Some(character) = self.peek() {
            self.position += character.len_utf8();
        }
    }

    fn rest(&self) -> &'a str {
        let source = self.source;
        &source[self.position..]
    }
}

fn punctuation(character: char) -> Option<TokenKind> {
    Some(match character {
        '[' => TokenKind::OpenBracket,
        ']' => TokenKind::CloseBracket,
        '(' => TokenKind::OpenParenthesis,
        ')' => TokenKind::CloseParenthesis,
        '{' => TokenKind::OpenBrace,
        '}' => TokenKind::CloseBrace,
        ',' => TokenKind::Comma,
        ':' => TokenKind::Colon,
        '*' => TokenKind::Star,
        '=' => TokenKind::Assign,
        _ => return None,
    })
}

fn keyword(name: &str) -> Option<TokenKind> {
    Some(match name {
        "BIT" => TokenKind::Bit,
        "NAND" => TokenKind::Nand,
        "BRANCH" => TokenKind::Branch,
        "EXTERN" => TokenKind::Extern,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        lex(source)
            .unwrap()
            .iter()
            .map(|token| token.kind().clone())
            .collect()
    }

    #[test]
    fn empty_source_has_no_tokens() {
        assert!(lex("").unwrap().is_empty());
    }

    #[test]
    fn punctuation_is_tokenized() {
        assert_eq!(
            kinds("[],:(){}*="),
            vec![
                TokenKind::OpenBracket,
                TokenKind::CloseBracket,
                TokenKind::Comma,
                TokenKind::Colon,
                TokenKind::OpenParenthesis,
                TokenKind::CloseParenthesis,
                TokenKind::OpenBrace,
                TokenKind::CloseBrace,
                TokenKind::Star,
                TokenKind::Assign,
            ]
        );
    }

    #[test]
    fn keywords_are_distinguished_from_labels() {
        assert_eq!(
            kinds("BIT NAND BRANCH EXTERN bit"),
            vec![
                TokenKind::Bit,
                TokenKind::Nand,
                TokenKind::Branch,
                TokenKind::Extern,
                TokenKind::Label("bit".to_owned()),
            ]
        );
    }

    #[test]
    fn labels_allow_infix_separators() {
        assert_eq!(
            kinds("cast.u2.to.pair"),
            vec![TokenKind::Label("cast.u2.to.pair".to_owned())]
        );
    }

    #[test]
    fn spans_cover_the_token() {
        let tokens = lex("  BIT").unwrap();
        assert_eq!(tokens[0].span(), Span::new(2, 5));
    }

    #[test]
    fn comments_are_skipped() {
        assert_eq!(
            kinds("BIT // NAND\nNAND"),
            vec![TokenKind::Bit, TokenKind::Nand]
        );
    }

    #[test]
    fn tokenizes_a_binding() {
        assert_eq!(
            kinds("state: BIT = ZERO"),
            vec![
                TokenKind::Label("state".to_owned()),
                TokenKind::Colon,
                TokenKind::Bit,
                TokenKind::Assign,
                TokenKind::Label("ZERO".to_owned()),
            ]
        );
    }

    #[test]
    fn unexpected_character_is_an_error() {
        let error = lex("BIT @").unwrap_err();
        assert_eq!(error.kind(), &LexErrorKind::UnexpectedCharacter('@'));
        assert_eq!(error.span(), Span::new(4, 5));
    }
}
