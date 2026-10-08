use std::fmt::Display;
use std::iter::Peekable;
use thiserror::Error;

use crate::tokenizer::{LiteralToken, OperatorToken, Token, Tokenizer, TokenizerError};

pub type Identifier = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Bit,
    Named(Identifier),
    Reference(Box<Type>),
    Collection(Vec<Type>),
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bit => write!(f, "BIT"),
            Self::Named(name) => write!(f, "{name}"),
            Self::Reference(inner) => write!(f, "*{inner}"),
            Self::Collection(elements) => {
                let formatted = elements
                    .iter()
                    .map(|element| element.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "[{formatted}]")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedIdentifier {
    pub name: Identifier,
    pub r#type: Option<Type>,
}

impl Display for TypedIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.r#type {
            Some(r#type) => write!(f, "{}: {type}", self.name),
            None => write!(f, "{}", self.name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Identifier(TypedIdentifier),
    Dereference(Identifier),
    Collection(Vec<Target>),
}

impl Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identifier(ident) => write!(f, "{ident}"),
            Self::Dereference(name) => write!(f, "*{name}"),
            Self::Collection(targets) => {
                let formatted = targets
                    .iter()
                    .map(|target| target.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "[{formatted}]")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NandOperation {
    pub left: Box<Expression>,
    pub right: Box<Expression>,
}

impl Display for NandOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NAND({}, {})", self.left, self.right)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchOperation {
    pub control: Box<Expression>,
    pub primary_branch: Block,
    pub secondary_branch: Block,
}

impl Display for BranchOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "BRANCH ({}) {} {}",
            self.control, self.primary_branch, self.secondary_branch
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCall {
    pub callee: Identifier,
    pub arguments: Vec<Expression>,
}

impl Display for FunctionCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let arguments = self
            .arguments
            .iter()
            .map(|argument| argument.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "{}({arguments})", self.callee)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerCall {
    pub arguments: Vec<Expression>,
}

impl Display for CompilerCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let arguments = self
            .arguments
            .iter()
            .map(|argument| argument.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "CALL({arguments})")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub name: Identifier,
    pub r#type: Type,
}

impl Display for Parameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.name, self.r#type)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDefinition {
    pub parameters: Vec<Parameter>,
    pub return_type: Type,
    pub body: Block,
}

impl Display for FunctionDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parameters = self
            .parameters
            .iter()
            .map(|parameter| parameter.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "({parameters}) {} {}", self.return_type, self.body)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expression {
    BitAllocation,
    Identifier(Identifier),
    Reference(Identifier),
    Nand(NandOperation),
    Branch(BranchOperation),
    FunctionCall(FunctionCall),
    CompilerCall(CompilerCall),
    Collection(Vec<Expression>),
    FunctionDefinition(FunctionDefinition),
}

impl Display for Expression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BitAllocation => write!(f, "BIT"),
            Self::Identifier(ident) => write!(f, "{ident}"),
            Self::Reference(ident) => write!(f, "*{ident}"),
            Self::Nand(nand) => write!(f, "{nand}"),
            Self::Branch(branch) => write!(f, "{branch}"),
            Self::FunctionCall(call) => write!(f, "{call}"),
            Self::CompilerCall(call) => write!(f, "{call}"),
            Self::Collection(elements) => {
                let formatted = elements
                    .iter()
                    .map(|element| element.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "[{formatted}]")
            }
            Self::FunctionDefinition(func) => write!(f, "{func}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub target: Target,
    pub value: Expression,
}

impl Display for Binding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} = {}", self.target, self.value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    Binding(Binding),
    Expression(Expression),
}

impl Display for Statement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Binding(binding) => write!(f, "{binding}"),
            Self::Expression(expression) => write!(f, "{expression}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub trailing_expression: Option<Box<Expression>>,
}

impl Display for Block {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.statements.is_empty() && self.trailing_expression.is_none() {
            return write!(f, "{{}}");
        }

        let statements = self
            .statements
            .iter()
            .map(|statement| format!("    {statement}\n"))
            .collect::<String>();

        let trailing = self
            .trailing_expression
            .as_ref()
            .map_or(String::new(), |expression| format!("    {expression}\n"));

        write!(f, "{{\n{statements}{trailing}}}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

impl Display for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for statement in &self.statements {
            writeln!(f, "{statement}")?;
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum TypeError {
    #[error("Unexpected End of Stream while parsing Type")]
    UnexpectedEndOfStream,
    #[error("Expected Type, found {0}")]
    ExpectedType(Token),
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum TargetError {
    #[error("Unexpected End of Stream while parsing Target")]
    UnexpectedEndOfStream,
    #[error("Expected Target, found {0}")]
    ExpectedTarget(Token),
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum ExpressionError {
    #[error("Unexpected End of Stream while parsing Expression")]
    UnexpectedEndOfStream,
    #[error("Expected Expression, found {0}")]
    ExpectedExpression(Token),
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum BlockError {
    #[error("Unexpected End of Stream while parsing Block")]
    UnexpectedEndOfStream,
    #[error("Expected '{{', found {0}")]
    ExpectedOpeningBrace(Token),
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum ParserError {
    #[error("Tokenizer Error: {0}")]
    TokenizerError(#[from] TokenizerError),
    #[error("Type Error: {0}")]
    TypeError(#[from] TypeError),
    #[error("Target Error: {0}")]
    TargetError(#[from] TargetError),
    #[error("Expression Error: {0}")]
    ExpressionError(#[from] ExpressionError),
    #[error("Block Error: {0}")]
    BlockError(#[from] BlockError),
    #[error("Expected Operator '{expected}', found {found:?}")]
    ExpectedOperator {
        expected: OperatorToken,
        found: Option<Token>,
    },
    #[error("Expected Identifier, found {0:?}")]
    ExpectedIdentifier(Option<Token>),
    #[error("Trailing Assignment Error: final statement in block cannot be an assignment")]
    TrailingAssignmentError,
}

pub struct Parser<'a> {
    tokenizer: Peekable<Tokenizer<'a>>,
}

impl<'a> Parser<'a> {
    pub fn new(tokenizer: Tokenizer<'a>) -> Self {
        Self {
            tokenizer: tokenizer.peekable(),
        }
    }

    fn peek_token(&mut self) -> Result<Option<&Token>, ParserError> {
        if matches!(self.tokenizer.peek(), Some(Err(_))) {
            let error = self.tokenizer.next().unwrap().unwrap_err();
            return Err(ParserError::TokenizerError(error));
        }

        match self.tokenizer.peek() {
            Some(Ok(token)) => Ok(Some(token)),
            None => Ok(None),
            Some(Err(_)) => unreachable!(),
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, ParserError> {
        self.tokenizer
            .next()
            .transpose()
            .map_err(ParserError::TokenizerError)
    }

    fn match_operator(&mut self, expected: OperatorToken) -> Result<bool, ParserError> {
        match self.peek_token()? {
            Some(Token::Operator(op)) if *op == expected => {
                self.next_token()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn expect_operator(&mut self, expected: OperatorToken) -> Result<(), ParserError> {
        match self.next_token()? {
            Some(Token::Operator(op)) if op == expected => Ok(()),
            found => Err(ParserError::ExpectedOperator { expected, found }),
        }
    }

    fn expect_identifier(&mut self) -> Result<Identifier, ParserError> {
        match self.next_token()? {
            Some(Token::Literal(LiteralToken::Label(ident))) => Ok(ident),
            found => Err(ParserError::ExpectedIdentifier(found)),
        }
    }

    fn is_binding(&self) -> bool {
        let mut depth_paren = 0;
        let mut depth_bracket = 0;
        let mut depth_brace = 0;

        for token in self.tokenizer.clone() {
            match token {
                Ok(Token::Operator(OperatorToken::LeftParenthesis)) => depth_paren += 1,
                Ok(Token::Operator(OperatorToken::RightParenthesis)) => depth_paren -= 1,
                Ok(Token::Operator(OperatorToken::LeftBracket)) => depth_bracket += 1,
                Ok(Token::Operator(OperatorToken::RightBracket)) => depth_bracket -= 1,
                Ok(Token::Operator(OperatorToken::LeftBrace)) => depth_brace += 1,
                Ok(Token::Operator(OperatorToken::RightBrace)) if depth_brace == 0 => return false,
                Ok(Token::Operator(OperatorToken::RightBrace)) => depth_brace -= 1,
                Ok(Token::Operator(OperatorToken::Equal))
                    if depth_paren == 0 && depth_bracket == 0 && depth_brace == 0 =>
                {
                    return true;
                }
                _ => (),
            }
        }
        false
    }

    fn parse_separated_sequence<T>(
        &mut self,
        closing_delimiter: OperatorToken,
        mut parse_element: impl FnMut(&mut Self) -> Result<T, ParserError>,
    ) -> Result<Vec<T>, ParserError> {
        match self.match_operator(closing_delimiter)? {
            true => Ok(Vec::new()),
            false => {
                let mut elements = Vec::new();
                loop {
                    elements.push(parse_element(self)?);
                    match self.match_operator(OperatorToken::Comma)? {
                        true if self.match_operator(closing_delimiter)? => break,
                        true => (),
                        false => {
                            self.expect_operator(closing_delimiter)?;
                            break;
                        }
                    }
                }
                Ok(elements)
            }
        }
    }

    pub fn parse_type(&mut self) -> Result<Type, ParserError> {
        match self.next_token()? {
            Some(Token::Literal(LiteralToken::Bit)) => Ok(Type::Bit),
            Some(Token::Literal(LiteralToken::Label(name))) => Ok(Type::Named(name)),
            Some(Token::Operator(OperatorToken::Star)) => {
                self.parse_type().map(Box::new).map(Type::Reference)
            }
            Some(Token::Operator(OperatorToken::LeftBracket)) => self
                .parse_separated_sequence(OperatorToken::RightBracket, Self::parse_type)
                .map(Type::Collection),
            Some(token) => Err(TypeError::ExpectedType(token).into()),
            None => Err(TypeError::UnexpectedEndOfStream.into()),
        }
    }

    pub fn parse_target(&mut self) -> Result<Target, ParserError> {
        match self.next_token()? {
            Some(Token::Operator(OperatorToken::Star)) => {
                self.expect_identifier().map(Target::Dereference)
            }
            Some(Token::Operator(OperatorToken::LeftBracket)) => self
                .parse_separated_sequence(OperatorToken::RightBracket, Self::parse_target)
                .map(Target::Collection),
            Some(Token::Literal(LiteralToken::Label(name))) => self
                .match_operator(OperatorToken::Colon)?
                .then(|| self.parse_type())
                .transpose()
                .map(|r#type| Target::Identifier(TypedIdentifier { name, r#type })),
            Some(token) => Err(TargetError::ExpectedTarget(token).into()),
            None => Err(TargetError::UnexpectedEndOfStream.into()),
        }
    }

    pub fn parse_expression(&mut self) -> Result<Expression, ParserError> {
        match self.next_token()? {
            Some(Token::Literal(LiteralToken::Bit)) => Ok(Expression::BitAllocation),
            Some(Token::Operator(OperatorToken::Star)) => {
                self.expect_identifier().map(Expression::Reference)
            }
            Some(Token::Literal(LiteralToken::Nand)) => {
                self.expect_operator(OperatorToken::LeftParenthesis)?;
                let left = self.parse_expression().map(Box::new)?;
                self.expect_operator(OperatorToken::Comma)?;
                let right = self.parse_expression().map(Box::new)?;
                self.expect_operator(OperatorToken::RightParenthesis)?;
                Ok(Expression::Nand(NandOperation { left, right }))
            }
            Some(Token::Literal(LiteralToken::Branch)) => {
                self.expect_operator(OperatorToken::LeftParenthesis)?;
                let control = self.parse_expression().map(Box::new)?;
                self.expect_operator(OperatorToken::RightParenthesis)?;
                let primary_branch = self.parse_block()?;
                let secondary_branch = self.parse_block()?;
                Ok(Expression::Branch(BranchOperation {
                    control,
                    primary_branch,
                    secondary_branch,
                }))
            }
            Some(Token::Literal(LiteralToken::Call)) => {
                self.expect_operator(OperatorToken::LeftParenthesis)?;
                self.parse_separated_sequence(
                    OperatorToken::RightParenthesis,
                    Self::parse_expression,
                )
                .map(|arguments| Expression::CompilerCall(CompilerCall { arguments }))
            }
            Some(Token::Operator(OperatorToken::LeftBracket)) => self
                .parse_separated_sequence(OperatorToken::RightBracket, Self::parse_expression)
                .map(Expression::Collection),
            Some(Token::Operator(OperatorToken::LeftParenthesis)) => {
                let parameters =
                    self.parse_separated_sequence(OperatorToken::RightParenthesis, |parser| {
                        let name = parser.expect_identifier()?;
                        parser.expect_operator(OperatorToken::Colon)?;
                        let r#type = parser.parse_type()?;
                        Ok(Parameter { name, r#type })
                    })?;
                let return_type = self.parse_type()?;
                let body = self.parse_block()?;
                Ok(Expression::FunctionDefinition(FunctionDefinition {
                    parameters,
                    return_type,
                    body,
                }))
            }
            Some(Token::Literal(LiteralToken::Label(callee)))
                if self.match_operator(OperatorToken::LeftParenthesis)? =>
            {
                self.parse_separated_sequence(
                    OperatorToken::RightParenthesis,
                    Self::parse_expression,
                )
                .map(|arguments| Expression::FunctionCall(FunctionCall { callee, arguments }))
            }
            Some(Token::Literal(LiteralToken::Label(name))) => Ok(Expression::Identifier(name)),
            Some(token) => Err(ExpressionError::ExpectedExpression(token).into()),
            None => Err(ExpressionError::UnexpectedEndOfStream.into()),
        }
    }

    pub fn parse_statement(&mut self) -> Result<Statement, ParserError> {
        match self.is_binding() {
            true => {
                let target = self.parse_target()?;
                self.expect_operator(OperatorToken::Equal)?;
                let value = self.parse_expression()?;
                Ok(Statement::Binding(Binding { target, value }))
            }
            false => self.parse_expression().map(Statement::Expression),
        }
    }

    pub fn parse_block(&mut self) -> Result<Block, ParserError> {
        self.expect_operator(OperatorToken::LeftBrace)?;

        match self.match_operator(OperatorToken::RightBrace)? {
            true => Ok(Block {
                statements: Vec::new(),
                trailing_expression: None,
            }),
            false => {
                let mut statements = Vec::new();
                let mut trailing_expression = None;

                while !self.match_operator(OperatorToken::RightBrace)? {
                    let statement = self.parse_statement()?;
                    match self.match_operator(OperatorToken::RightBrace)? {
                        true => match statement {
                            Statement::Expression(expression) => {
                                trailing_expression = Some(Box::new(expression));
                                break;
                            }
                            Statement::Binding(_) => {
                                return Err(ParserError::TrailingAssignmentError);
                            }
                        },
                        false => statements.push(statement),
                    }
                }

                Ok(Block {
                    statements,
                    trailing_expression,
                })
            }
        }
    }

    pub fn parse_program(&mut self) -> Result<Program, ParserError> {
        let mut statements = Vec::new();
        while self.peek_token()?.is_some() {
            statements.push(self.parse_statement()?);
        }
        Ok(Program { statements })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_type() {
        assert_eq!(
            Parser::new(Tokenizer::new("BIT")).parse_type(),
            Ok(Type::Bit)
        );
        assert_eq!(
            Parser::new(Tokenizer::new("*PAIR")).parse_type(),
            Ok(Type::Reference(Box::new(Type::Named("PAIR".to_string()))))
        );
        assert_eq!(
            Parser::new(Tokenizer::new("[BIT, *U2]")).parse_type(),
            Ok(Type::Collection(vec![
                Type::Bit,
                Type::Reference(Box::new(Type::Named("U2".to_string()))),
            ]))
        );
    }

    #[test]
    fn test_parse_target() {
        assert_eq!(
            Parser::new(Tokenizer::new("x: BIT")).parse_target(),
            Ok(Target::Identifier(TypedIdentifier {
                name: "x".to_string(),
                r#type: Some(Type::Bit),
            }))
        );
        assert_eq!(
            Parser::new(Tokenizer::new("*slot")).parse_target(),
            Ok(Target::Dereference("slot".to_string()))
        );
        assert_eq!(
            Parser::new(Tokenizer::new("[a, *b]")).parse_target(),
            Ok(Target::Collection(vec![
                Target::Identifier(TypedIdentifier {
                    name: "a".to_string(),
                    r#type: None,
                }),
                Target::Dereference("b".to_string()),
            ]))
        );
    }

    #[test]
    fn test_parse_expression() {
        assert_eq!(
            Parser::new(Tokenizer::new("NAND(a, *b)")).parse_expression(),
            Ok(Expression::Nand(NandOperation {
                left: Box::new(Expression::Identifier("a".to_string())),
                right: Box::new(Expression::Reference("b".to_string())),
            }))
        );
        assert_eq!(
            Parser::new(Tokenizer::new("CALL(OP.ALLOC, size)")).parse_expression(),
            Ok(Expression::CompilerCall(CompilerCall {
                arguments: vec![
                    Expression::Identifier("OP.ALLOC".to_string()),
                    Expression::Identifier("size".to_string()),
                ],
            }))
        );
    }

    #[test]
    fn test_parse_block() {
        let src = "{
            dummy: BIT = ZERO
            opt.b
        }";
        let block = Parser::new(Tokenizer::new(src)).parse_block().unwrap();
        assert_eq!(block.statements.len(), 1);
        assert_eq!(
            block.trailing_expression,
            Some(Box::new(Expression::Identifier("opt.b".to_string())))
        );
    }

    #[test]
    fn test_parse_block_invalid_trailing_assignment() {
        let src = "{ dummy = ZERO }";
        assert_eq!(
            Parser::new(Tokenizer::new(src)).parse_block(),
            Err(ParserError::TrailingAssignmentError)
        );
    }

    #[test]
    fn test_parse_statement() {
        let binding = Parser::new(Tokenizer::new("flag: BIT = ONE")).parse_statement();
        assert_eq!(
            binding,
            Ok(Statement::Binding(Binding {
                target: Target::Identifier(TypedIdentifier {
                    name: "flag".to_string(),
                    r#type: Some(Type::Bit),
                }),
                value: Expression::Identifier("ONE".to_string()),
            }))
        );

        let expression = Parser::new(Tokenizer::new("compute(state)")).parse_statement();
        assert_eq!(
            expression,
            Ok(Statement::Expression(Expression::FunctionCall(
                FunctionCall {
                    callee: "compute".to_string(),
                    arguments: vec![Expression::Identifier("state".to_string())],
                }
            )))
        );
    }

    #[test]
    fn test_parse_program() {
        let src = "
            ONE: BIT = NAND(seed, seed)
            toggle = (target: *BIT) [] {
                target = NAND(target, target)
                []
            }
        ";
        let program = Parser::new(Tokenizer::new(src)).parse_program();
        assert!(program.is_ok());
        assert_eq!(program.unwrap().statements.len(), 2);
    }
}
