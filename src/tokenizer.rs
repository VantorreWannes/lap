use std::fmt::Display;

use thiserror::Error;

#[derive(Debug, PartialEq, Eq, Error)]
pub enum OperatorError {
    #[error("Invalid Operator Error: {0}")]
    InvalidOperatorError(String),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum OperatorToken {
    Equal,
    Star,
    Colon,
    Comma,
    LeftParenthesis,
    RightParenthesis,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Dot,
}

impl OperatorToken {
    pub fn new(value: &str) -> Result<Self, OperatorError> {
        match value {
            "=" => Ok(Self::Equal),
            "*" => Ok(Self::Star),
            ":" => Ok(Self::Colon),
            "," => Ok(Self::Comma),
            "." => Ok(Self::Dot),
            "(" => Ok(Self::LeftParenthesis),
            ")" => Ok(Self::RightParenthesis),
            "[" => Ok(Self::LeftBracket),
            "]" => Ok(Self::RightBracket),
            "{" => Ok(Self::LeftBrace),
            "}" => Ok(Self::RightBrace),
            _ => Err(OperatorError::InvalidOperatorError(value.to_string())),
        }
    }
}

impl TryFrom<&str> for OperatorToken {
    type Error = OperatorError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Display for OperatorToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Equal => write!(f, "="),
            Self::Star => write!(f, "*"),
            Self::Colon => write!(f, ":"),
            Self::Comma => write!(f, ","),
            Self::Dot => write!(f, "."),
            Self::LeftParenthesis => write!(f, "("),
            Self::RightParenthesis => write!(f, ")"),
            Self::LeftBracket => write!(f, "["),
            Self::RightBracket => write!(f, "]"),
            Self::LeftBrace => write!(f, "{}", '{'),
            Self::RightBrace => write!(f, "{}", '}'),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum LiteralError {
    #[error("Invalid Literal Error: {0}")]
    InvalidLiteralError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiteralToken {
    Bit,
    Nand,
    Branch,
    Sys,
    Std,
    Label(String),
}

impl LiteralToken {
    fn is_label_str(value: &str) -> bool {
        !value.is_empty()
            && value
                .chars()
                .all(|value_char| value_char.is_ascii_alphanumeric() || value_char == '_')
    }

    pub fn new(value: &str) -> Result<Self, LiteralError> {
        match value {
            "BIT" => Ok(Self::Bit),
            "NAND" => Ok(Self::Nand),
            "BRANCH" => Ok(Self::Branch),
            "SYS" => Ok(Self::Sys),
            "STD" => Ok(Self::Std),
            value if Self::is_label_str(value) => Ok(Self::Label(value.to_string())),
            _ => Err(LiteralError::InvalidLiteralError(value.to_string())),
        }
    }
}

impl TryFrom<&str> for LiteralToken {
    type Error = LiteralError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Display for LiteralToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bit => write!(f, "BIT"),
            Self::Nand => write!(f, "NAND"),
            Self::Branch => write!(f, "BRANCH"),
            Self::Sys => write!(f, "SYS"),
            Self::Std => write!(f, "STD"),
            Self::Label(label) => write!(f, "{label}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum TokenError {
    #[error("Invalid Token: {0}")]
    InvalidTokenError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Operator(OperatorToken),
    Literal(LiteralToken),
}

impl Token {
    pub fn new(value: &str) -> Result<Self, TokenError> {
        if let Ok(operator) = OperatorToken::new(value) {
            return Ok(Self::Operator(operator));
        }

        if let Ok(literal) = LiteralToken::new(value) {
            return Ok(Self::Literal(literal));
        }

        Err(TokenError::InvalidTokenError(value.to_string()))
    }
}

impl TryFrom<&str> for Token {
    type Error = TokenError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Operator(operator) => write!(f, "{operator}"),
            Token::Literal(literal) => write!(f, "{literal}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum TokenizerError {
    #[error("Token Error: {0}")]
    TokenError(#[from] TokenError),
}

#[derive(Debug, Clone)]
pub struct Tokenizer<'a> {
    src: &'a str,
}

impl<'a> Tokenizer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self { src }
    }

    fn skip_whitespace(&mut self) {
        self.src = self.src.trim_start();
    }

    fn skip_comment(&mut self) {
        if self.src.starts_with("//") {
            let end = self.src.find('\n').unwrap_or(self.src.len());
            self.src = &self.src[end..];
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            let before = self.src.len();
            self.skip_whitespace();
            self.skip_comment();
            if self.src.len() == before {
                break;
            }
        }
    }

    pub fn next_token(&mut self) -> Option<Result<Token, TokenizerError>> {
        self.skip_whitespace_and_comments();
        for end in (1..=self.src.len()).rev() {
            if !self.src.is_char_boundary(end) {
                continue;
            }

            if let Ok(token) = Token::try_from(&self.src[..end]) {
                self.src = &self.src[end..];
                return Some(Ok(token));
            }
        }
        None
    }
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = Result<Token, TokenizerError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operator_token_valid_new_ok() {
        assert_eq!(OperatorToken::new("="), Ok(OperatorToken::Equal));
        assert_eq!(OperatorToken::new("*"), Ok(OperatorToken::Star));
        assert_eq!(OperatorToken::new(":"), Ok(OperatorToken::Colon));
        assert_eq!(OperatorToken::new(","), Ok(OperatorToken::Comma));
        assert_eq!(OperatorToken::new("."), Ok(OperatorToken::Dot));
        assert_eq!(OperatorToken::new("("), Ok(OperatorToken::LeftParenthesis));
        assert_eq!(OperatorToken::new(")"), Ok(OperatorToken::RightParenthesis));
        assert_eq!(OperatorToken::new("["), Ok(OperatorToken::LeftBracket));
        assert_eq!(OperatorToken::new("]"), Ok(OperatorToken::RightBracket));
        assert_eq!(OperatorToken::new("{"), Ok(OperatorToken::LeftBrace));
        assert_eq!(OperatorToken::new("}"), Ok(OperatorToken::RightBrace));
    }

    #[test]
    fn test_operator_token_new_invalid() {
        assert_eq!(
            OperatorToken::new(""),
            Err(OperatorError::InvalidOperatorError("".to_owned()))
        );
        assert_eq!(
            OperatorToken::new(" "),
            Err(OperatorError::InvalidOperatorError(" ".to_owned()))
        );
        assert_eq!(
            OperatorToken::new("A"),
            Err(OperatorError::InvalidOperatorError("A".to_owned()))
        );
        assert_eq!(
            OperatorToken::new("=="),
            Err(OperatorError::InvalidOperatorError("==".to_owned()))
        );
    }

    #[test]
    fn test_literal_token_valid_new_ok() {
        assert_eq!(LiteralToken::new("BIT"), Ok(LiteralToken::Bit));
        assert_eq!(LiteralToken::new("NAND"), Ok(LiteralToken::Nand));
        assert_eq!(LiteralToken::new("BRANCH"), Ok(LiteralToken::Branch));
        assert_eq!(LiteralToken::new("STD"), Ok(LiteralToken::Std));
        assert_eq!(LiteralToken::new("SYS"), Ok(LiteralToken::Sys));
        assert_eq!(
            LiteralToken::new("u1"),
            Ok(LiteralToken::Label("u1".to_string()))
        );
        assert_eq!(
            LiteralToken::new("Value"),
            Ok(LiteralToken::Label("Value".to_string()))
        );
        assert_eq!(
            LiteralToken::new("INDEX"),
            Ok(LiteralToken::Label("INDEX".to_string()))
        );
    }

    #[test]
    fn test_tokenizer() {
        let src = " zero = BIT.zero ";
        let mut tokenizer = Tokenizer::new(src);
        {
            let expected = Token::new("zero").unwrap();
            assert_eq!(tokenizer.next_token(), Some(Ok(expected)));
        }
        {
            let expected = Token::new("=").unwrap();
            assert_eq!(tokenizer.next_token(), Some(Ok(expected)));
        }
        {
            let expected = Token::new("BIT").unwrap();
            assert_eq!(tokenizer.next_token(), Some(Ok(expected)));
        }
        {
            let expected = Token::new(".").unwrap();
            assert_eq!(tokenizer.next_token(), Some(Ok(expected)));
        }
        {
            let expected = Token::new("zero").unwrap();
            assert_eq!(tokenizer.next_token(), Some(Ok(expected)));
        }
    }
}
