#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Label(String);

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        assert!(!text.is_empty(), "a label is not empty");
        Self(text)
    }

    pub fn text(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    statements: Vec<Statement>,
}

impl Program {
    pub fn new(statements: Vec<Statement>) -> Self {
        Self { statements }
    }

    pub fn statements(&self) -> &[Statement] {
        &self.statements
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Statement {
    Binding(Binding),
    Expression(Expression),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    target: Target,
    value: BindingValue,
}

impl Binding {
    pub fn new(target: Target, value: BindingValue) -> Self {
        Self { target, value }
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn value(&self) -> &BindingValue {
        &self.value
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingValue {
    Expression(Expression),
    Function(FunctionDefinition),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Named(NamedTarget),
    Reference(Label),
    Destructuring(Vec<Target>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedTarget {
    label: Label,
    type_expression: Option<TypeExpression>,
}

impl NamedTarget {
    pub fn new(label: Label, type_expression: Option<TypeExpression>) -> Self {
        Self {
            label,
            type_expression,
        }
    }

    pub fn label(&self) -> &Label {
        &self.label
    }

    pub fn type_expression(&self) -> Option<&TypeExpression> {
        self.type_expression.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionDefinition {
    parameters: Vec<Parameter>,
    result: TypeExpression,
    body: Block,
}

impl FunctionDefinition {
    pub fn new(parameters: Vec<Parameter>, result: TypeExpression, body: Block) -> Self {
        Self {
            parameters,
            result,
            body,
        }
    }

    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    pub fn result(&self) -> &TypeExpression {
        &self.result
    }

    pub fn body(&self) -> &Block {
        &self.body
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameter {
    label: Label,
    type_expression: TypeExpression,
}

impl Parameter {
    pub fn new(label: Label, type_expression: TypeExpression) -> Self {
        Self {
            label,
            type_expression,
        }
    }

    pub fn label(&self) -> &Label {
        &self.label
    }

    pub fn type_expression(&self) -> &TypeExpression {
        &self.type_expression
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    statements: Vec<Statement>,
    result: Option<Box<Expression>>,
}

impl Block {
    pub fn new(statements: Vec<Statement>, result: Option<Box<Expression>>) -> Self {
        Self { statements, result }
    }

    pub fn statements(&self) -> &[Statement] {
        &self.statements
    }

    pub fn result(&self) -> Option<&Expression> {
        self.result.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expression {
    Bit,
    Label(Label),
    Reference(Label),
    Nand(Box<Expression>, Box<Expression>),
    Branch(Box<Expression>, Block, Block),
    Extern(Vec<Expression>),
    Collection(Vec<Expression>),
    Call(Label, Vec<Expression>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeExpression {
    Bit,
    Label(Label),
    Reference(Box<TypeExpression>),
    Collection(Vec<TypeExpression>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_keeps_text() {
        let label = Label::new("bit.not");
        assert_eq!(label.text(), "bit.not");
    }

    #[test]
    fn program_keeps_statements() {
        let program = Program::new(vec![Statement::Expression(Expression::Bit)]);
        assert_eq!(program.statements().len(), 1);
    }

    #[test]
    fn binding_keeps_target_and_value() {
        let target = Target::Named(NamedTarget::new(Label::new("state"), None));
        let binding = Binding::new(target, BindingValue::Expression(Expression::Bit));
        assert_eq!(
            binding.target(),
            &Target::Named(NamedTarget::new(Label::new("state"), None))
        );
    }

    #[test]
    fn function_definition_keeps_parameters_result_and_body() {
        let parameter = Parameter::new(Label::new("a"), TypeExpression::Bit);
        let body = Block::new(vec![], Some(Box::new(Expression::Bit)));
        let function = FunctionDefinition::new(vec![parameter], TypeExpression::Bit, body);
        assert_eq!(function.parameters().len(), 1);
        assert_eq!(function.result(), &TypeExpression::Bit);
        assert!(function.body().result().is_some());
    }

    #[test]
    fn block_keeps_statements_and_result() {
        let block = Block::new(
            vec![Statement::Expression(Expression::Bit)],
            Some(Box::new(Expression::Bit)),
        );
        assert_eq!(block.statements().len(), 1);
        assert_eq!(block.result(), Some(&Expression::Bit));
    }

    #[test]
    #[should_panic]
    fn an_empty_label_is_rejected() {
        Label::new("");
    }

    #[test]
    fn named_target_keeps_its_parts() {
        let target = NamedTarget::new(Label::new("state"), Some(TypeExpression::Bit));
        assert_eq!(target.label().text(), "state");
        assert_eq!(target.type_expression(), Some(&TypeExpression::Bit));
    }

    #[test]
    fn named_target_without_a_type_has_none() {
        let target = NamedTarget::new(Label::new("state"), None);
        assert_eq!(target.type_expression(), None);
    }

    #[test]
    fn parameter_keeps_its_parts() {
        let parameter = Parameter::new(Label::new("a"), TypeExpression::Bit);
        assert_eq!(parameter.label().text(), "a");
        assert_eq!(parameter.type_expression(), &TypeExpression::Bit);
    }

    #[test]
    fn nested_targets_keep_their_shape() {
        let target = Target::Destructuring(vec![
            Target::Named(NamedTarget::new(Label::new("a"), None)),
            Target::Reference(Label::new("b")),
        ]);
        match target {
            Target::Destructuring(targets) => assert_eq!(targets.len(), 2),
            _ => panic!("the target is a destructuring"),
        }
    }
}
