use lapc_ast::Program;

pub fn parse_program(source: &str) -> Result<Program, ParseError> {
    let tokens = lexer::tokenize(source)?;
    parser::parse_program(tokens, Position::new(source.len()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position(usize);

impl Position {
    pub fn new(offset: usize) -> Self {
        Self(offset)
    }

    pub fn offset(&self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseErrorKind {
    UnexpectedCharacter(char),
    InvalidLabel,
    UnexpectedToken,
    UnexpectedEndOfInput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    position: Position,
    kind: ParseErrorKind,
}

impl ParseError {
    pub fn new(position: Position, kind: ParseErrorKind) -> Self {
        Self { position, kind }
    }

    pub fn position(&self) -> Position {
        self.position
    }

    pub fn kind(&self) -> ParseErrorKind {
        self.kind
    }
}

mod lexer {
    use super::{ParseError, ParseErrorKind, Position};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Keyword {
        Bit,
        Nand,
        Branch,
        Extern,
    }

    impl Keyword {
        pub(super) fn from_text(text: &str) -> Option<Self> {
            match text {
                "BIT" => Some(Self::Bit),
                "NAND" => Some(Self::Nand),
                "BRANCH" => Some(Self::Branch),
                "EXTERN" => Some(Self::Extern),
                _ => None,
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Punctuation {
        LeftParenthesis,
        RightParenthesis,
        LeftBracket,
        RightBracket,
        LeftBrace,
        RightBrace,
        Comma,
        Colon,
        Equals,
        Star,
    }

    impl Punctuation {
        fn from_character(character: char) -> Option<Self> {
            match character {
                '(' => Some(Self::LeftParenthesis),
                ')' => Some(Self::RightParenthesis),
                '[' => Some(Self::LeftBracket),
                ']' => Some(Self::RightBracket),
                '{' => Some(Self::LeftBrace),
                '}' => Some(Self::RightBrace),
                ',' => Some(Self::Comma),
                ':' => Some(Self::Colon),
                '=' => Some(Self::Equals),
                '*' => Some(Self::Star),
                _ => None,
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum TokenKind<'a> {
        Label(&'a str),
        Keyword(Keyword),
        Punctuation(Punctuation),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct Token<'a> {
        kind: TokenKind<'a>,
        position: Position,
    }

    impl<'a> Token<'a> {
        fn new(kind: TokenKind<'a>, position: Position) -> Self {
            Self { kind, position }
        }

        pub(super) fn kind(&self) -> TokenKind<'a> {
            self.kind
        }

        pub(super) fn position(&self) -> Position {
            self.position
        }
    }

    pub(super) fn tokenize(source: &str) -> Result<Vec<Token<'_>>, ParseError> {
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

        fn tokenize(mut self) -> Result<Vec<Token<'a>>, ParseError> {
            let mut tokens = Vec::new();
            while let Some(character) = self.peek() {
                if character.is_whitespace() {
                    self.advance();
                    continue;
                }
                if self.remaining().starts_with("//") {
                    self.skip_comment();
                    continue;
                }
                let position = Position::new(self.position);
                let kind = self.lex_token(character, position)?;
                tokens.push(Token::new(kind, position));
            }
            Ok(tokens)
        }

        fn lex_token(
            &mut self,
            character: char,
            position: Position,
        ) -> Result<TokenKind<'a>, ParseError> {
            if let Some(punctuation) = Punctuation::from_character(character) {
                self.advance();
                return Ok(TokenKind::Punctuation(punctuation));
            }
            if character.is_ascii_alphanumeric() {
                return self.lex_label();
            }
            Err(ParseError::new(
                position,
                ParseErrorKind::UnexpectedCharacter(character),
            ))
        }

        fn lex_label(&mut self) -> Result<TokenKind<'a>, ParseError> {
            let start = self.position;
            self.advance_while(|character| character.is_ascii_alphanumeric());
            while let Some('_' | '.') = self.peek() {
                let separator = self.position;
                self.advance();
                if !self
                    .peek()
                    .is_some_and(|character| character.is_ascii_alphanumeric())
                {
                    return Err(ParseError::new(
                        Position::new(separator),
                        ParseErrorKind::InvalidLabel,
                    ));
                }
                self.advance_while(|character| character.is_ascii_alphanumeric());
            }
            let text = &self.source[start..self.position];
            Ok(match Keyword::from_text(text) {
                Some(keyword) => TokenKind::Keyword(keyword),
                None => TokenKind::Label(text),
            })
        }

        fn skip_comment(&mut self) {
            while self.peek().is_some_and(|character| character != '\n') {
                self.advance();
            }
        }

        fn peek(&self) -> Option<char> {
            self.remaining().chars().next()
        }

        fn remaining(&self) -> &'a str {
            &self.source[self.position..]
        }

        fn advance(&mut self) {
            if let Some(character) = self.peek() {
                self.position += character.len_utf8();
            }
        }

        fn advance_while(&mut self, predicate: impl Fn(char) -> bool) {
            while self.peek().is_some_and(&predicate) {
                self.advance();
            }
        }
    }
}

mod parser {
    use super::lexer::{Keyword, Punctuation, Token, TokenKind};
    use super::{ParseError, ParseErrorKind, Position};
    use lapc_ast::{
        Binding, BindingValue, Block, Expression, FunctionDefinition, Label, NamedTarget,
        Parameter, Program, Statement, Target, TypeExpression,
    };

    pub(super) fn parse_program(
        tokens: Vec<Token<'_>>,
        end_position: Position,
    ) -> Result<Program, ParseError> {
        Parser::new(tokens, end_position).parse_program()
    }

    struct Parser<'a> {
        tokens: Vec<Token<'a>>,
        index: usize,
        end_position: Position,
    }

    impl<'a> Parser<'a> {
        fn new(tokens: Vec<Token<'a>>, end_position: Position) -> Self {
            Self {
                tokens,
                index: 0,
                end_position,
            }
        }

        fn parse_program(mut self) -> Result<Program, ParseError> {
            let mut statements = Vec::new();
            while self.current().is_some() {
                statements.push(self.parse_statement()?);
            }
            Ok(Program::new(statements))
        }

        fn parse_statement(&mut self) -> Result<Statement, ParseError> {
            let checkpoint = self.index;
            if let Ok(target) = self.parse_target()
                && self.current_punctuation() == Some(Punctuation::Equals)
            {
                self.index += 1;
                let value = self.parse_binding_value()?;
                return Ok(Statement::Binding(Binding::new(target, value)));
            }
            self.index = checkpoint;
            Ok(Statement::Expression(self.parse_expression()?))
        }

        fn parse_binding_value(&mut self) -> Result<BindingValue, ParseError> {
            if self.current_punctuation() == Some(Punctuation::LeftParenthesis) {
                Ok(BindingValue::Function(self.parse_function_definition()?))
            } else {
                Ok(BindingValue::Expression(self.parse_expression()?))
            }
        }

        fn parse_target(&mut self) -> Result<Target, ParseError> {
            match self.current_kind() {
                Some(TokenKind::Punctuation(Punctuation::Star)) => {
                    self.index += 1;
                    Ok(Target::Reference(self.parse_label()?))
                }
                Some(TokenKind::Punctuation(Punctuation::LeftBracket)) => {
                    self.index += 1;
                    let targets = self.parse_sequence(Punctuation::RightBracket, |parser| {
                        parser.parse_target()
                    })?;
                    if targets.is_empty() {
                        return Err(self.unexpected_token());
                    }
                    self.expect_punctuation(Punctuation::RightBracket)?;
                    Ok(Target::Destructuring(targets))
                }
                Some(TokenKind::Label(text)) => {
                    self.index += 1;
                    let label = Label::new(text);
                    let type_expression = if self.current_punctuation() == Some(Punctuation::Colon)
                    {
                        self.index += 1;
                        Some(self.parse_type()?)
                    } else {
                        None
                    };
                    Ok(Target::Named(NamedTarget::new(label, type_expression)))
                }
                _ => Err(self.unexpected_token()),
            }
        }

        fn parse_function_definition(&mut self) -> Result<FunctionDefinition, ParseError> {
            self.expect_punctuation(Punctuation::LeftParenthesis)?;
            let parameters = self.parse_sequence(Punctuation::RightParenthesis, |parser| {
                parser.parse_parameter()
            })?;
            self.expect_punctuation(Punctuation::RightParenthesis)?;
            let result = self.parse_type()?;
            let body = self.parse_block()?;
            Ok(FunctionDefinition::new(parameters, result, body))
        }

        fn parse_parameter(&mut self) -> Result<Parameter, ParseError> {
            let label = self.parse_label()?;
            self.expect_punctuation(Punctuation::Colon)?;
            let type_expression = self.parse_type()?;
            Ok(Parameter::new(label, type_expression))
        }

        fn parse_type(&mut self) -> Result<TypeExpression, ParseError> {
            match self.current_kind() {
                Some(TokenKind::Keyword(Keyword::Bit)) => {
                    self.index += 1;
                    Ok(TypeExpression::Bit)
                }
                Some(TokenKind::Punctuation(Punctuation::Star)) => {
                    self.index += 1;
                    Ok(TypeExpression::Reference(Box::new(self.parse_type()?)))
                }
                Some(TokenKind::Punctuation(Punctuation::LeftBracket)) => {
                    self.index += 1;
                    let elements = self
                        .parse_sequence(Punctuation::RightBracket, |parser| parser.parse_type())?;
                    self.expect_punctuation(Punctuation::RightBracket)?;
                    Ok(TypeExpression::Collection(elements))
                }
                Some(TokenKind::Label(text)) => {
                    self.index += 1;
                    Ok(TypeExpression::Label(Label::new(text)))
                }
                _ => Err(self.unexpected_token()),
            }
        }

        fn parse_expression(&mut self) -> Result<Expression, ParseError> {
            match self.current_kind() {
                Some(TokenKind::Keyword(Keyword::Bit)) => {
                    self.index += 1;
                    Ok(Expression::Bit)
                }
                Some(TokenKind::Keyword(Keyword::Nand)) => self.parse_nand(),
                Some(TokenKind::Keyword(Keyword::Branch)) => self.parse_branch(),
                Some(TokenKind::Keyword(Keyword::Extern)) => self.parse_extern(),
                Some(TokenKind::Punctuation(Punctuation::Star)) => {
                    self.index += 1;
                    Ok(Expression::Reference(self.parse_label()?))
                }
                Some(TokenKind::Punctuation(Punctuation::LeftBracket)) => self.parse_collection(),
                Some(TokenKind::Label(text)) => {
                    self.index += 1;
                    let label = Label::new(text);
                    if self.current_punctuation() == Some(Punctuation::LeftParenthesis) {
                        self.parse_call(label)
                    } else {
                        Ok(Expression::Label(label))
                    }
                }
                _ => Err(self.unexpected_token()),
            }
        }

        fn parse_nand(&mut self) -> Result<Expression, ParseError> {
            self.index += 1;
            self.expect_punctuation(Punctuation::LeftParenthesis)?;
            let left = self.parse_expression()?;
            self.expect_punctuation(Punctuation::Comma)?;
            let right = self.parse_expression()?;
            self.expect_punctuation(Punctuation::RightParenthesis)?;
            Ok(Expression::Nand(Box::new(left), Box::new(right)))
        }

        fn parse_branch(&mut self) -> Result<Expression, ParseError> {
            self.index += 1;
            self.expect_punctuation(Punctuation::LeftParenthesis)?;
            let condition = self.parse_expression()?;
            self.expect_punctuation(Punctuation::RightParenthesis)?;
            let when_true = self.parse_block()?;
            let when_false = self.parse_block()?;
            Ok(Expression::Branch(
                Box::new(condition),
                when_true,
                when_false,
            ))
        }

        fn parse_extern(&mut self) -> Result<Expression, ParseError> {
            self.index += 1;
            self.expect_punctuation(Punctuation::LeftParenthesis)?;
            let arguments = self.parse_sequence(Punctuation::RightParenthesis, |parser| {
                parser.parse_expression()
            })?;
            if arguments.is_empty() {
                return Err(self.unexpected_token());
            }
            self.expect_punctuation(Punctuation::RightParenthesis)?;
            Ok(Expression::Extern(arguments))
        }

        fn parse_collection(&mut self) -> Result<Expression, ParseError> {
            self.index += 1;
            let elements = self.parse_sequence(Punctuation::RightBracket, |parser| {
                parser.parse_expression()
            })?;
            self.expect_punctuation(Punctuation::RightBracket)?;
            Ok(Expression::Collection(elements))
        }

        fn parse_call(&mut self, label: Label) -> Result<Expression, ParseError> {
            self.expect_punctuation(Punctuation::LeftParenthesis)?;
            let arguments = self.parse_sequence(Punctuation::RightParenthesis, |parser| {
                parser.parse_expression()
            })?;
            self.expect_punctuation(Punctuation::RightParenthesis)?;
            Ok(Expression::Call(label, arguments))
        }

        fn parse_block(&mut self) -> Result<Block, ParseError> {
            self.expect_punctuation(Punctuation::LeftBrace)?;
            let mut statements = Vec::new();
            let mut result = None;
            while self.current_punctuation() != Some(Punctuation::RightBrace) {
                let statement = self.parse_statement()?;
                if self.current_punctuation() == Some(Punctuation::RightBrace) {
                    if let Statement::Expression(expression) = statement {
                        result = Some(Box::new(expression));
                    } else {
                        statements.push(statement);
                    }
                    break;
                }
                statements.push(statement);
            }
            self.expect_punctuation(Punctuation::RightBrace)?;
            Ok(Block::new(statements, result))
        }

        fn parse_sequence<T>(
            &mut self,
            closing: Punctuation,
            mut parse_element: impl FnMut(&mut Self) -> Result<T, ParseError>,
        ) -> Result<Vec<T>, ParseError> {
            let mut elements = Vec::new();
            if self.current_punctuation() == Some(closing) {
                return Ok(elements);
            }
            loop {
                elements.push(parse_element(self)?);
                if self.current_punctuation() != Some(Punctuation::Comma) {
                    return Ok(elements);
                }
                self.index += 1;
            }
        }

        fn parse_label(&mut self) -> Result<Label, ParseError> {
            match self.current_kind() {
                Some(TokenKind::Label(text)) => {
                    self.index += 1;
                    Ok(Label::new(text))
                }
                _ => Err(self.unexpected_token()),
            }
        }

        fn expect_punctuation(&mut self, expected: Punctuation) -> Result<(), ParseError> {
            if self.current_punctuation() == Some(expected) {
                self.index += 1;
                Ok(())
            } else {
                Err(self.unexpected_token())
            }
        }

        fn current(&self) -> Option<Token<'a>> {
            self.tokens.get(self.index).copied()
        }

        fn current_kind(&self) -> Option<TokenKind<'a>> {
            self.current().map(|token| token.kind())
        }

        fn current_punctuation(&self) -> Option<Punctuation> {
            match self.current_kind() {
                Some(TokenKind::Punctuation(punctuation)) => Some(punctuation),
                _ => None,
            }
        }

        fn unexpected_token(&self) -> ParseError {
            match self.current() {
                Some(token) => ParseError::new(token.position(), ParseErrorKind::UnexpectedToken),
                None => ParseError::new(self.end_position, ParseErrorKind::UnexpectedEndOfInput),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lapc_ast::{
        Binding, BindingValue, Block, Expression, FunctionDefinition, Label, NamedTarget,
        Parameter, Statement, Target, TypeExpression,
    };

    fn parse(source: &str) -> Program {
        parse_program(source).expect("the source parses")
    }

    fn statements(source: &str) -> Vec<Statement> {
        parse(source).statements().to_vec()
    }

    fn parse_error(source: &str) -> ParseError {
        parse_program(source).expect_err("the source does not parse")
    }

    fn binding_of(statement: &Statement) -> &Binding {
        match statement {
            Statement::Binding(binding) => binding,
            Statement::Expression(_) => panic!("the statement is a binding"),
        }
    }

    fn function_of_binding(binding: &Binding) -> &FunctionDefinition {
        match binding.value() {
            BindingValue::Function(function) => function,
            BindingValue::Expression(_) => panic!("the binding value is a function"),
        }
    }

    fn label(text: &str) -> Expression {
        Expression::Label(Label::new(text))
    }

    fn named_binding(text: &str, value: Expression) -> Statement {
        Statement::Binding(Binding::new(
            Target::Named(NamedTarget::new(Label::new(text), None)),
            BindingValue::Expression(value),
        ))
    }

    fn readme_example() -> &'static str {
        let readme = include_str!("../../../README.md");
        let section = readme
            .split("## Example")
            .nth(1)
            .expect("the README has an example section");
        section
            .split("```")
            .nth(1)
            .expect("the example section has a code block")
    }

    #[test]
    fn readme_example_parses() {
        let program = parse(readme_example());
        assert_eq!(program.statements().len(), 12);

        let first = binding_of(&program.statements()[0]);
        assert_eq!(
            first.target(),
            &Target::Named(NamedTarget::new(Label::new("BIT.ONE"), None))
        );
        assert_eq!(
            first.value(),
            &BindingValue::Expression(Expression::Nand(
                Box::new(Expression::Bit),
                Box::new(Expression::Nand(
                    Box::new(Expression::Bit),
                    Box::new(Expression::Bit),
                )),
            ))
        );

        let main = function_of_binding(binding_of(
            program
                .statements()
                .last()
                .expect("the example has statements"),
        ));
        assert_eq!(main.parameters().len(), 0);
        assert_eq!(main.result(), &TypeExpression::Bit);
        assert!(main.body().result().is_some());
    }

    #[test]
    fn bit_expression_parses() {
        assert_eq!(
            statements("BIT"),
            vec![Statement::Expression(Expression::Bit)]
        );
    }

    #[test]
    fn label_expression_parses() {
        assert_eq!(
            statements("BIT.ONE"),
            vec![Statement::Expression(label("BIT.ONE"))]
        );
    }

    #[test]
    fn reference_expression_parses() {
        assert_eq!(
            statements("*state"),
            vec![Statement::Expression(Expression::Reference(Label::new(
                "state"
            )))]
        );
    }

    #[test]
    fn nand_expression_parses() {
        assert_eq!(
            statements("NAND(BIT, BIT.ONE)"),
            vec![Statement::Expression(Expression::Nand(
                Box::new(Expression::Bit),
                Box::new(Expression::Label(Label::new("BIT.ONE"))),
            ))]
        );
    }

    #[test]
    fn branch_expression_parses() {
        assert_eq!(
            statements("BRANCH (flag) { a } { b }"),
            vec![Statement::Expression(Expression::Branch(
                Box::new(Expression::Label(Label::new("flag"))),
                Block::new(vec![], Some(Box::new(label("a")))),
                Block::new(vec![], Some(Box::new(label("b")))),
            ))]
        );
    }

    #[test]
    fn extern_expression_parses() {
        assert_eq!(
            statements("EXTERN(OP.MEMORY.ACQUIRE, handle)"),
            vec![Statement::Expression(Expression::Extern(vec![
                Expression::Label(Label::new("OP.MEMORY.ACQUIRE")),
                Expression::Label(Label::new("handle")),
            ]))]
        );
    }

    #[test]
    fn collection_expression_parses() {
        assert_eq!(
            statements("[BIT, BIT.ONE]"),
            vec![Statement::Expression(Expression::Collection(vec![
                Expression::Bit,
                Expression::Label(Label::new("BIT.ONE")),
            ]))]
        );
    }

    #[test]
    fn empty_collection_expression_parses() {
        assert_eq!(
            statements("[]"),
            vec![Statement::Expression(Expression::Collection(vec![]))]
        );
    }

    #[test]
    fn call_expression_parses() {
        assert_eq!(
            statements("bit.not(a)"),
            vec![Statement::Expression(Expression::Call(
                Label::new("bit.not"),
                vec![Expression::Label(Label::new("a"))],
            ))]
        );
    }

    #[test]
    fn call_without_arguments_parses() {
        assert_eq!(
            statements("heap.alloc()"),
            vec![Statement::Expression(Expression::Call(
                Label::new("heap.alloc"),
                vec![],
            ))]
        );
    }

    #[test]
    fn binding_with_type_parses() {
        assert_eq!(
            statements("state: BIT = BIT.ZERO"),
            vec![Statement::Binding(Binding::new(
                Target::Named(NamedTarget::new(
                    Label::new("state"),
                    Some(TypeExpression::Bit),
                )),
                BindingValue::Expression(Expression::Label(Label::new("BIT.ZERO"))),
            ))]
        );
    }

    #[test]
    fn binding_without_type_parses() {
        assert_eq!(
            statements("x = BIT"),
            vec![Statement::Binding(Binding::new(
                Target::Named(NamedTarget::new(Label::new("x"), None)),
                BindingValue::Expression(Expression::Bit),
            ))]
        );
    }

    #[test]
    fn reference_target_binding_parses() {
        assert_eq!(
            statements("*state = BIT.ONE"),
            vec![Statement::Binding(Binding::new(
                Target::Reference(Label::new("state")),
                BindingValue::Expression(Expression::Label(Label::new("BIT.ONE"))),
            ))]
        );
    }

    #[test]
    fn destructuring_binding_parses() {
        assert_eq!(
            statements("[head, *middle, last] = bundle"),
            vec![Statement::Binding(Binding::new(
                Target::Destructuring(vec![
                    Target::Named(NamedTarget::new(Label::new("head"), None)),
                    Target::Reference(Label::new("middle")),
                    Target::Named(NamedTarget::new(Label::new("last"), None)),
                ]),
                BindingValue::Expression(Expression::Label(Label::new("bundle"))),
            ))]
        );
    }

    #[test]
    fn function_definition_parses() {
        assert_eq!(
            statements("bit.not = (a: BIT) BIT { NAND(a, a) }"),
            vec![Statement::Binding(Binding::new(
                Target::Named(NamedTarget::new(Label::new("bit.not"), None)),
                BindingValue::Function(FunctionDefinition::new(
                    vec![Parameter::new(Label::new("a"), TypeExpression::Bit)],
                    TypeExpression::Bit,
                    Block::new(
                        vec![],
                        Some(Box::new(Expression::Nand(
                            Box::new(Expression::Label(Label::new("a"))),
                            Box::new(Expression::Label(Label::new("a"))),
                        ))),
                    ),
                )),
            ))]
        );
    }

    #[test]
    fn function_without_parameters_parses() {
        assert_eq!(
            statements("heap.alloc = () [BIT, U64] { EXTERN(OP.MEMORY.ACQUIRE) }"),
            vec![Statement::Binding(Binding::new(
                Target::Named(NamedTarget::new(Label::new("heap.alloc"), None)),
                BindingValue::Function(FunctionDefinition::new(
                    vec![],
                    TypeExpression::Collection(vec![
                        TypeExpression::Bit,
                        TypeExpression::Label(Label::new("U64")),
                    ]),
                    Block::new(
                        vec![],
                        Some(Box::new(Expression::Extern(vec![Expression::Label(
                            Label::new("OP.MEMORY.ACQUIRE"),
                        )]))),
                    ),
                )),
            ))]
        );
    }

    #[test]
    fn block_result_is_the_trailing_expression() {
        let program = parse("f = () BIT { x = BIT y }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        assert_eq!(function.body().statements().len(), 1);
        assert_eq!(function.body().result(), Some(&label("y")));
    }

    #[test]
    fn block_ending_in_binding_has_no_result() {
        let program = parse("f = () [] { x = BIT }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        assert_eq!(function.body().statements().len(), 1);
        assert_eq!(function.body().result(), None);
    }

    #[test]
    fn empty_block_has_no_result() {
        let program = parse("f = () [] { }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        assert!(function.body().statements().is_empty());
        assert_eq!(function.body().result(), None);
    }

    #[test]
    fn comments_and_whitespace_are_stripped() {
        assert_eq!(
            statements("  // leading\nBIT // trailing\n"),
            vec![Statement::Expression(Expression::Bit)]
        );
    }

    #[test]
    fn nested_types_parse() {
        assert_eq!(
            statements("x: [BIT, *[BIT]] = y"),
            vec![Statement::Binding(Binding::new(
                Target::Named(NamedTarget::new(
                    Label::new("x"),
                    Some(TypeExpression::Collection(vec![
                        TypeExpression::Bit,
                        TypeExpression::Reference(Box::new(TypeExpression::Collection(vec![
                            TypeExpression::Bit,
                        ]))),
                    ])),
                )),
                BindingValue::Expression(Expression::Label(Label::new("y"))),
            ))]
        );
    }

    #[test]
    fn nested_function_definition_parses() {
        let program = parse("f = () [] { g = () [] { } g() }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        assert_eq!(function.body().statements().len(), 1);
        assert_eq!(
            function.body().result(),
            Some(&Expression::Call(Label::new("g"), vec![]))
        );
    }

    #[test]
    fn unexpected_character_reports_its_position() {
        let error = parse_program("BIT @").expect_err("the source does not parse");
        assert_eq!(error.position(), Position::new(4));
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedCharacter('@'));
    }

    #[test]
    fn unexpected_end_of_input_reports_its_position() {
        let error = parse_program("x = ").expect_err("the source does not parse");
        assert_eq!(error.position(), Position::new(4));
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedEndOfInput);
    }

    #[test]
    fn invalid_label_reports_its_position() {
        let error = parse_program("BIT. = BIT").expect_err("the source does not parse");
        assert_eq!(error.position(), Position::new(3));
        assert_eq!(error.kind(), ParseErrorKind::InvalidLabel);
    }

    #[test]
    fn keyword_is_not_a_target() {
        let error = parse_program("BIT = BIT").expect_err("the source does not parse");
        assert_eq!(error.position(), Position::new(4));
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
    }

    #[test]
    fn empty_input_parses_to_an_empty_program() {
        assert!(statements("").is_empty());
    }

    #[test]
    fn whitespace_only_input_parses_to_an_empty_program() {
        assert!(statements(" \t\r\n ").is_empty());
    }

    #[test]
    fn comment_only_input_parses_to_an_empty_program() {
        assert!(statements("// leading comment").is_empty());
        assert!(statements("// comment without a line ending").is_empty());
    }

    #[test]
    fn a_keyword_is_a_keyword_only_when_the_whole_label_equals_it() {
        for text in [
            "BITX",
            "BIT.ONE",
            "BIT_ONE",
            "NANDY",
            "NAND.NAND",
            "BRANCHING",
            "BRANCH.x",
            "EXTERNALLY",
            "EXTERN.x",
            "Bit",
            "nand",
        ] {
            assert_eq!(
                statements(text),
                vec![Statement::Expression(label(text))],
                "the label {text} is not a keyword"
            );
        }
    }

    #[test]
    fn every_keyword_is_rejected_where_a_label_is_required() {
        for (source, offset) in [
            ("BIT = BIT", 4),
            ("NAND = BIT", 5),
            ("BRANCH = BIT", 7),
            ("EXTERN = BIT", 7),
        ] {
            let error = parse_error(source);
            assert_eq!(
                error.kind(),
                ParseErrorKind::UnexpectedToken,
                "for {source}"
            );
            assert_eq!(error.position(), Position::new(offset), "for {source}");
        }
    }

    #[test]
    fn a_keyword_is_rejected_as_a_parameter_label() {
        let error = parse_error("f = (BIT: BIT) [] {}");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(5));
    }

    #[test]
    fn a_keyword_is_rejected_as_a_type() {
        let error = parse_error("f = () NAND {}");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(7));
    }

    #[test]
    fn comment_runs_to_the_end_of_its_line() {
        assert_eq!(
            statements("BIT // BIT\nBIT"),
            vec![
                Statement::Expression(Expression::Bit),
                Statement::Expression(Expression::Bit),
            ]
        );
    }

    #[test]
    fn comments_are_stripped_between_every_token() {
        assert_eq!(
            statements("NAND( // left\n BIT, // right\n BIT // last\n )"),
            vec![Statement::Expression(Expression::Nand(
                Box::new(Expression::Bit),
                Box::new(Expression::Bit),
            ))]
        );
    }

    #[test]
    fn a_comment_may_follow_a_token_without_whitespace() {
        assert_eq!(
            statements("BIT// trailing"),
            vec![Statement::Expression(Expression::Bit)]
        );
    }

    #[test]
    fn carriage_returns_are_whitespace() {
        assert_eq!(
            statements("BIT\r\nBIT"),
            vec![
                Statement::Expression(Expression::Bit),
                Statement::Expression(Expression::Bit),
            ]
        );
    }

    #[test]
    fn labels_may_contain_digits_underscores_and_dots() {
        assert_eq!(statements("123"), vec![Statement::Expression(label("123"))]);
        assert_eq!(
            statements("a_b.c1"),
            vec![Statement::Expression(label("a_b.c1"))]
        );
    }

    #[test]
    fn a_separator_must_be_followed_by_an_alphanumeric() {
        for (source, offset) in [
            ("a.", 1),
            ("a_", 1),
            ("a..b", 1),
            ("a._b", 1),
            ("a. b", 1),
            ("BIT_", 3),
            ("BIT. ONE", 3),
        ] {
            let error = parse_error(source);
            assert_eq!(error.kind(), ParseErrorKind::InvalidLabel, "for {source}");
            assert_eq!(error.position(), Position::new(offset), "for {source}");
        }
    }

    #[test]
    fn a_separator_is_not_a_valid_label_start() {
        for (source, character) in [("_a", '_'), (".a", '.')] {
            let error = parse_error(source);
            assert_eq!(
                error.kind(),
                ParseErrorKind::UnexpectedCharacter(character),
                "for {source}"
            );
            assert_eq!(error.position(), Position::new(0), "for {source}");
        }
    }

    #[test]
    fn unexpected_character_position_is_a_byte_offset() {
        assert_eq!("BIT é".len(), 6);
        let error = parse_error("BIT é");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedCharacter('é'));
        assert_eq!(error.position(), Position::new(4));
    }

    #[test]
    fn dotted_reference_expression_parses() {
        assert_eq!(
            statements("*a.b"),
            vec![Statement::Expression(Expression::Reference(Label::new(
                "a.b"
            )))]
        );
    }

    #[test]
    fn nested_nand_expression_parses() {
        assert_eq!(
            statements("NAND(NAND(BIT, a), NAND(b, c))"),
            vec![Statement::Expression(Expression::Nand(
                Box::new(Expression::Nand(
                    Box::new(Expression::Bit),
                    Box::new(label("a")),
                )),
                Box::new(Expression::Nand(Box::new(label("b")), Box::new(label("c")),)),
            ))]
        );
    }

    #[test]
    fn nand_requires_exactly_two_arguments() {
        for (source, offset) in [
            ("NAND()", 5),
            ("NAND(BIT)", 8),
            ("NAND(BIT,)", 9),
            ("NAND(BIT, BIT, BIT)", 13),
        ] {
            let error = parse_error(source);
            assert_eq!(
                error.kind(),
                ParseErrorKind::UnexpectedToken,
                "for {source}"
            );
            assert_eq!(error.position(), Position::new(offset), "for {source}");
        }
    }

    #[test]
    fn branch_blocks_parse_without_whitespace() {
        assert_eq!(
            statements("BRANCH(flag){a}{b}"),
            vec![Statement::Expression(Expression::Branch(
                Box::new(label("flag")),
                Block::new(vec![], Some(Box::new(label("a")))),
                Block::new(vec![], Some(Box::new(label("b")))),
            ))]
        );
    }

    #[test]
    fn branch_requires_a_condition_and_two_blocks() {
        let error = parse_error("BRANCH{}");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(6));

        let error = parse_error("BRANCH(x){}");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedEndOfInput);
        assert_eq!(error.position(), Position::new(11));

        let error = parse_error("BRANCH(x){}{}{}");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(13));
    }

    #[test]
    fn extern_accepts_several_and_nested_arguments() {
        assert_eq!(
            statements("EXTERN(a, NAND(b, c), [d])"),
            vec![Statement::Expression(Expression::Extern(vec![
                label("a"),
                Expression::Nand(Box::new(label("b")), Box::new(label("c"))),
                Expression::Collection(vec![label("d")]),
            ]))]
        );
    }

    #[test]
    fn extern_without_arguments_is_rejected() {
        let error = parse_error("EXTERN()");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(7));
    }

    #[test]
    fn nested_collection_expression_parses() {
        assert_eq!(
            statements("[a, [b, [c]]]"),
            vec![Statement::Expression(Expression::Collection(vec![
                label("a"),
                Expression::Collection(vec![label("b"), Expression::Collection(vec![label("c")]),]),
            ]))]
        );
    }

    #[test]
    fn collection_rejects_a_trailing_comma() {
        let error = parse_error("[a,]");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(3));
    }

    #[test]
    fn nested_call_expression_parses() {
        assert_eq!(
            statements("f(g(), h(i()))"),
            vec![Statement::Expression(Expression::Call(
                Label::new("f"),
                vec![
                    Expression::Call(Label::new("g"), vec![]),
                    Expression::Call(
                        Label::new("h"),
                        vec![Expression::Call(Label::new("i"), vec![])],
                    ),
                ],
            ))]
        );
    }

    #[test]
    fn call_rejects_a_trailing_comma() {
        let error = parse_error("f(a,)");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(4));
    }

    #[test]
    fn a_reference_is_not_callable() {
        let error = parse_error("*a(b)");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(2));
    }

    #[test]
    fn statements_are_separated_by_tokens_not_by_newlines() {
        assert_eq!(
            statements("BIT BIT"),
            vec![
                Statement::Expression(Expression::Bit),
                Statement::Expression(Expression::Bit),
            ]
        );
        assert_eq!(statements("a = BIT b = BIT").len(), 2);
    }

    #[test]
    fn a_binding_value_may_be_a_reference_a_call_or_a_collection() {
        assert_eq!(
            statements("x = *a"),
            vec![named_binding("x", Expression::Reference(Label::new("a")))]
        );
        assert_eq!(
            statements("x = f()"),
            vec![named_binding(
                "x",
                Expression::Call(Label::new("f"), vec![])
            )]
        );
        assert_eq!(
            statements("x = [a]"),
            vec![named_binding("x", Expression::Collection(vec![label("a")]))]
        );
    }

    #[test]
    fn a_typed_target_may_bind_a_function_definition() {
        let program = parse("f: myType = (a: BIT) BIT { a }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        assert_eq!(function.body().result(), Some(&label("a")));
    }

    #[test]
    fn typed_binding_targets_may_be_reference_or_collection_types() {
        assert_eq!(
            statements("x: *[BIT] = y"),
            vec![Statement::Binding(Binding::new(
                Target::Named(NamedTarget::new(
                    Label::new("x"),
                    Some(TypeExpression::Reference(Box::new(
                        TypeExpression::Collection(vec![TypeExpression::Bit]),
                    ))),
                )),
                BindingValue::Expression(label("y")),
            ))]
        );
    }

    #[test]
    fn nested_destructuring_target_parses() {
        assert_eq!(
            statements("[[a: *BIT], *b] = c"),
            vec![Statement::Binding(Binding::new(
                Target::Destructuring(vec![
                    Target::Destructuring(vec![Target::Named(NamedTarget::new(
                        Label::new("a"),
                        Some(TypeExpression::Reference(Box::new(TypeExpression::Bit))),
                    ))]),
                    Target::Reference(Label::new("b")),
                ]),
                BindingValue::Expression(label("c")),
            ))]
        );
    }

    #[test]
    fn empty_destructuring_target_is_rejected() {
        let error = parse_error("[] = x");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(3));
    }

    #[test]
    fn a_collection_expression_that_is_not_a_target_is_not_a_binding() {
        assert_eq!(
            statements("[[]]"),
            vec![Statement::Expression(Expression::Collection(vec![
                Expression::Collection(vec![]),
            ]))]
        );
    }

    #[test]
    fn a_target_without_a_binding_falls_back_to_an_expression() {
        assert_eq!(
            statements("[a] b"),
            vec![
                Statement::Expression(Expression::Collection(vec![label("a")])),
                Statement::Expression(label("b")),
            ]
        );
    }

    #[test]
    fn a_typed_target_without_equals_is_not_a_binding() {
        let error = parse_error("x: BIT");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(1));
    }

    #[test]
    fn dotted_reference_target_binding_parses() {
        assert_eq!(
            statements("*a.b = c"),
            vec![Statement::Binding(Binding::new(
                Target::Reference(Label::new("a.b")),
                BindingValue::Expression(label("c")),
            ))]
        );
    }

    #[test]
    fn function_with_several_parameters_parses() {
        assert_eq!(
            statements("f = (a: BIT, b: [BIT, *[]]) [] { a }"),
            vec![Statement::Binding(Binding::new(
                Target::Named(NamedTarget::new(Label::new("f"), None)),
                BindingValue::Function(FunctionDefinition::new(
                    vec![
                        Parameter::new(Label::new("a"), TypeExpression::Bit),
                        Parameter::new(
                            Label::new("b"),
                            TypeExpression::Collection(vec![
                                TypeExpression::Bit,
                                TypeExpression::Reference(Box::new(TypeExpression::Collection(
                                    vec![],
                                ))),
                            ]),
                        ),
                    ],
                    TypeExpression::Collection(vec![]),
                    Block::new(vec![], Some(Box::new(label("a")))),
                )),
            ))]
        );
    }

    #[test]
    fn function_parameter_requires_a_colon_and_a_type() {
        let error = parse_error("f = (a) [] {}");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(6));

        let error = parse_error("f = (a BIT) [] {}");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(7));
    }

    #[test]
    fn function_definition_requires_a_result_type_and_a_body() {
        let error = parse_error("f = () {}");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedToken);
        assert_eq!(error.position(), Position::new(7));

        let error = parse_error("f = () []");
        assert_eq!(error.kind(), ParseErrorKind::UnexpectedEndOfInput);
        assert_eq!(error.position(), Position::new(9));
    }

    #[test]
    fn nested_function_definition_keeps_inner_shapes() {
        let program = parse("f = () [] { g = (a: BIT) BIT { a } }");
        let outer = function_of_binding(binding_of(&program.statements()[0]));
        assert_eq!(outer.body().statements().len(), 1);
        assert_eq!(outer.body().result(), None);

        let inner = function_of_binding(binding_of(&outer.body().statements()[0]));
        assert_eq!(inner.parameters().len(), 1);
        assert_eq!(inner.result(), &TypeExpression::Bit);
        assert_eq!(inner.body().result(), Some(&label("a")));
    }

    #[test]
    fn a_block_ending_in_a_function_binding_has_no_result() {
        let program = parse("f = () [] { g = () [] {} }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        assert_eq!(function.body().statements().len(), 1);
        assert_eq!(function.body().result(), None);
    }

    #[test]
    fn a_block_keeps_non_trailing_expression_statements() {
        let program = parse("f = () [] { a b }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        assert_eq!(function.body().statements().len(), 1);
        assert_eq!(
            function.body().statements()[0],
            Statement::Expression(label("a"))
        );
        assert_eq!(function.body().result(), Some(&label("b")));
    }

    #[test]
    fn a_block_ending_in_a_destructuring_binding_has_no_result() {
        let program = parse("f = () [] { [a] = b }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        assert_eq!(function.body().statements().len(), 1);
        assert_eq!(function.body().result(), None);
    }

    #[test]
    fn a_block_result_may_be_a_reference_or_a_call() {
        let program = parse("f = () [] { g = () [] {} *a }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        assert_eq!(
            function.body().result(),
            Some(&Expression::Reference(Label::new("a")))
        );

        let program = parse("f = () [] { g() }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        assert_eq!(
            function.body().result(),
            Some(&Expression::Call(Label::new("g"), vec![]))
        );
    }

    #[test]
    fn nested_blocks_report_their_own_results() {
        let program = parse("f = () [] { BRANCH (x) { a } { b = y c } }");
        let function = function_of_binding(binding_of(&program.statements()[0]));
        let Some(Expression::Branch(_, when_true, when_false)) = function.body().result() else {
            panic!("the block result is a branch");
        };
        assert_eq!(when_true.result(), Some(&label("a")));
        assert_eq!(when_false.statements().len(), 1);
        assert_eq!(when_false.result(), Some(&label("c")));
    }

    #[test]
    fn unterminated_input_reports_the_end_of_input() {
        for source in [
            "x =",
            "x = (",
            "NAND(BIT",
            "EXTERN(x",
            "*",
            "[",
            "f = () [] {",
            "f = () [] { x",
        ] {
            let error = parse_error(source);
            assert_eq!(
                error.kind(),
                ParseErrorKind::UnexpectedEndOfInput,
                "for {source}"
            );
            assert_eq!(
                error.position(),
                Position::new(source.len()),
                "for {source}"
            );
        }
    }

    fn assert_label_is_not_a_keyword(label: &Label) {
        assert!(
            lexer::Keyword::from_text(label.text()).is_none(),
            "the label {} is a keyword",
            label.text()
        );
    }

    fn assert_type_invariants(type_expression: &TypeExpression) {
        match type_expression {
            TypeExpression::Bit => (),
            TypeExpression::Label(label) => assert_label_is_not_a_keyword(label),
            TypeExpression::Reference(inner) => assert_type_invariants(inner),
            TypeExpression::Collection(elements) => {
                for element in elements {
                    assert_type_invariants(element);
                }
            }
        }
    }

    fn assert_target_invariants(target: &Target) {
        match target {
            Target::Named(named) => {
                assert_label_is_not_a_keyword(named.label());
                if let Some(type_expression) = named.type_expression() {
                    assert_type_invariants(type_expression);
                }
            }
            Target::Reference(label) => assert_label_is_not_a_keyword(label),
            Target::Destructuring(targets) => {
                for target in targets {
                    assert_target_invariants(target);
                }
            }
        }
    }

    fn assert_block_invariants(block: &Block) {
        if block.result().is_none()
            && let Some(statement) = block.statements().last()
        {
            assert!(
                matches!(statement, Statement::Binding(_)),
                "a block without a result ends in a binding"
            );
        }
        for statement in block.statements() {
            assert_statement_invariants(statement);
        }
        if let Some(result) = block.result() {
            assert_expression_invariants(result);
        }
    }

    fn assert_expression_invariants(expression: &Expression) {
        match expression {
            Expression::Bit => (),
            Expression::Label(label) | Expression::Reference(label) => {
                assert_label_is_not_a_keyword(label);
            }
            Expression::Nand(left, right) => {
                assert_expression_invariants(left);
                assert_expression_invariants(right);
            }
            Expression::Branch(condition, when_true, when_false) => {
                assert_expression_invariants(condition);
                assert_block_invariants(when_true);
                assert_block_invariants(when_false);
            }
            Expression::Extern(arguments) | Expression::Collection(arguments) => {
                for argument in arguments {
                    assert_expression_invariants(argument);
                }
            }
            Expression::Call(label, arguments) => {
                assert_label_is_not_a_keyword(label);
                for argument in arguments {
                    assert_expression_invariants(argument);
                }
            }
        }
    }

    fn assert_binding_invariants(binding: &Binding) {
        assert_target_invariants(binding.target());
        match binding.value() {
            BindingValue::Expression(expression) => assert_expression_invariants(expression),
            BindingValue::Function(function) => {
                for parameter in function.parameters() {
                    assert_label_is_not_a_keyword(parameter.label());
                    assert_type_invariants(parameter.type_expression());
                }
                assert_type_invariants(function.result());
                assert_block_invariants(function.body());
            }
        }
    }

    fn assert_statement_invariants(statement: &Statement) {
        match statement {
            Statement::Binding(binding) => assert_binding_invariants(binding),
            Statement::Expression(expression) => assert_expression_invariants(expression),
        }
    }

    fn assert_program_invariants(program: &Program) {
        for statement in program.statements() {
            assert_statement_invariants(statement);
        }
    }

    #[test]
    fn parsed_programs_hold_the_label_and_block_invariants() {
        assert_program_invariants(&parse(readme_example()));
        assert_program_invariants(&parse("f = () [] { BRANCH (x) { a } { y = b c } [d] = e }"));
        assert_program_invariants(&parse("[a, *b] = EXTERN(OP, *c)"));
    }

    #[test]
    fn every_error_position_lies_within_the_source() {
        let alphabet: Vec<char> = "BITNANDARX()[]{},:=*._0a/ \n@é".chars().collect();
        let mut state: u64 = 0x2545F4914F6CDD1D;
        for _ in 0..100_000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let length = (state >> 33) as usize % 12;
            let mut source = String::new();
            for _ in 0..length {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                source.push(alphabet[(state >> 33) as usize % alphabet.len()]);
            }
            match parse_program(&source) {
                Ok(program) => assert_program_invariants(&program),
                Err(error) => {
                    assert!(error.position().offset() <= source.len(), "for {source:?}");
                }
            }
        }
    }
}
