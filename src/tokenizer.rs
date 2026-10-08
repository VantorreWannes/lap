use std::fmt::{self, Display};

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
}

impl OperatorToken {
    pub const fn as_char(self) -> char {
        match self {
            Self::Equal => '=',
            Self::Star => '*',
            Self::Colon => ':',
            Self::Comma => ',',
            Self::LeftParenthesis => '(',
            Self::RightParenthesis => ')',
            Self::LeftBracket => '[',
            Self::RightBracket => ']',
            Self::LeftBrace => '{',
            Self::RightBrace => '}',
        }
    }

    pub const fn from_char(c: char) -> Option<Self> {
        match c {
            '=' => Some(Self::Equal),
            '*' => Some(Self::Star),
            ':' => Some(Self::Colon),
            ',' => Some(Self::Comma),
            '(' => Some(Self::LeftParenthesis),
            ')' => Some(Self::RightParenthesis),
            '[' => Some(Self::LeftBracket),
            ']' => Some(Self::RightBracket),
            '{' => Some(Self::LeftBrace),
            '}' => Some(Self::RightBrace),
            _ => None,
        }
    }

    pub fn new(value: &str) -> Result<Self, OperatorError> {
        let invalid = || OperatorError::InvalidOperatorError(value.to_owned());
        let mut chars = value.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Self::from_char(c).ok_or_else(invalid),
            _ => Err(invalid()),
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_char())
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
    Extern,
    Label(String),
}

pub const fn is_label_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.'
}

fn is_label_str(value: &str) -> bool {
    !value.is_empty()
        && value.split(['_', '.']).all(|segment| {
            !segment.is_empty() && segment.chars().all(|c| c.is_ascii_alphanumeric())
        })
}

impl LiteralToken {
    pub fn new(value: &str) -> Result<Self, LiteralError> {
        match value {
            "BIT" => Ok(Self::Bit),
            "NAND" => Ok(Self::Nand),
            "BRANCH" => Ok(Self::Branch),
            "EXTERN" => Ok(Self::Extern),
            _ if is_label_str(value) => Ok(Self::Label(value.to_owned())),
            _ => Err(LiteralError::InvalidLiteralError(value.to_owned())),
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bit => write!(f, "BIT"),
            Self::Nand => write!(f, "NAND"),
            Self::Branch => write!(f, "BRANCH"),
            Self::Extern => write!(f, "EXTERN"),
            Self::Label(label) => write!(f, "{label}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
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
        OperatorToken::new(value)
            .map(Self::Operator)
            .or_else(|_| LiteralToken::new(value).map(Self::Literal))
            .map_err(|_| TokenError::InvalidTokenError(value.to_owned()))
    }
}

impl TryFrom<&str> for Token {
    type Error = TokenError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operator(operator) => write!(f, "{operator}"),
            Self::Literal(literal) => write!(f, "{literal}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TokenizerError {
    #[error("unexpected character {0:?}")]
    UnexpectedCharacter(char),

    #[error("malformed label `{0}`")]
    MalformedLabel(String),
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

    fn skip_line_comment(&mut self) -> bool {
        match self.src.strip_prefix("//") {
            Some(comment) => {
                let end = comment.find('\n').map_or(comment.len(), |nl| nl + 1);
                self.src = &comment[end..];
                true
            }
            None => false,
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            self.skip_whitespace();
            if !self.skip_line_comment() {
                return;
            }
        }
    }

    pub fn next_token(&mut self) -> Option<Result<Token, TokenizerError>> {
        self.skip_trivia();

        let first = self.src.chars().next()?;

        if let Some(operator) = OperatorToken::from_char(first) {
            self.src = &self.src[first.len_utf8()..];
            return Some(Ok(Token::Operator(operator)));
        }

        if first.is_ascii_alphanumeric() {
            let len = self.src.len() - self.src.trim_start_matches(is_label_char).len();
            let word = &self.src[..len];
            self.src = &self.src[len..];

            return Some(match LiteralToken::new(word) {
                Ok(literal) => Ok(Token::Literal(literal)),
                Err(_) => Err(TokenizerError::MalformedLabel(word.to_owned())),
            });
        }

        self.src = &self.src[first.len_utf8()..];
        Some(Err(TokenizerError::UnexpectedCharacter(first)))
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
        assert_eq!(
            OperatorToken::new("."),
            Err(OperatorError::InvalidOperatorError(".".to_owned()))
        );
    }

    #[test]
    fn test_literal_token_valid_new_ok() {
        assert_eq!(LiteralToken::new("BIT"), Ok(LiteralToken::Bit));
        assert_eq!(LiteralToken::new("NAND"), Ok(LiteralToken::Nand));
        assert_eq!(LiteralToken::new("BRANCH"), Ok(LiteralToken::Branch));
        assert_eq!(LiteralToken::new("EXTERN"), Ok(LiteralToken::Extern));
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
        assert_eq!(
            LiteralToken::new("BIT.zero"),
            Ok(LiteralToken::Label("BIT.zero".to_string()))
        );
        assert_eq!(
            LiteralToken::new("cast.u2.to.pair"),
            Ok(LiteralToken::Label("cast.u2.to.pair".to_string()))
        );
        assert_eq!(
            LiteralToken::new("system_state_1"),
            Ok(LiteralToken::Label("system_state_1".to_string()))
        );
    }

    #[test]
    fn test_literal_token_invalid_labels() {
        assert!(LiteralToken::new("_label").is_err());
        assert!(LiteralToken::new("label_").is_err());
        assert!(LiteralToken::new(".label").is_err());
        assert!(LiteralToken::new("label.").is_err());
        assert!(LiteralToken::new("a..b").is_err());
        assert!(LiteralToken::new("a._b").is_err());
        assert!(LiteralToken::new("").is_err());
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
            let expected = Token::new("BIT.zero").unwrap();
            assert_eq!(tokenizer.next_token(), Some(Ok(expected)));
        }
        assert_eq!(tokenizer.next_token(), None);
    }

    #[test]
    fn test_tokenizer_no_whitespace() {
        let mut tokenizer = Tokenizer::new("a=BIT");
        assert_eq!(tokenizer.next_token(), Some(Ok(Token::new("a").unwrap())));
        assert_eq!(tokenizer.next_token(), Some(Ok(Token::new("=").unwrap())));
        assert_eq!(tokenizer.next_token(), Some(Ok(Token::new("BIT").unwrap())));
        assert_eq!(tokenizer.next_token(), None);
    }

    #[test]
    fn test_tokenizer_reports_unknown_character() {
        let mut tokenizer = Tokenizer::new("zero = ;foo");
        assert_eq!(
            tokenizer.next_token(),
            Some(Ok(Token::new("zero").unwrap()))
        );
        assert_eq!(tokenizer.next_token(), Some(Ok(Token::new("=").unwrap())));
        assert_eq!(
            tokenizer.next_token(),
            Some(Err(TokenizerError::UnexpectedCharacter(';')))
        );
    }

    #[test]
    fn test_tokenizer_rejects_malformed_label() {
        let mut tokenizer = Tokenizer::new("a..b");
        assert_eq!(
            tokenizer.next_token(),
            Some(Err(TokenizerError::MalformedLabel("a..b".to_owned())))
        );
    }

    #[test]
    fn test_tokenizer_skips_comments() {
        let mut tokenizer = Tokenizer::new("//\nfoo // trailing\nBAR");
        assert_eq!(
            tokenizer.next_token(),
            Some(Ok(Token::Literal(LiteralToken::Label("foo".to_owned()))))
        );
        assert_eq!(
            tokenizer.next_token(),
            Some(Ok(Token::Literal(LiteralToken::Label("BAR".to_owned()))))
        );
        assert_eq!(tokenizer.next_token(), None);
    }

    #[test]
    fn test_tokenizer_iterator_collect() {
        let tokens: Result<Vec<_>, _> = Tokenizer::new("main = () BIT").collect();
        assert_eq!(
            tokens,
            Ok(vec![
                Token::new("main").unwrap(),
                Token::new("=").unwrap(),
                Token::new("(").unwrap(),
                Token::new(")").unwrap(),
                Token::new("BIT").unwrap(),
            ])
        );
    }
}
