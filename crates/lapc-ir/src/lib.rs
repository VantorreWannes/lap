use lapc_ast::Label;
use lapc_extern::Operation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Bit,
    Reference(Box<Type>),
    Collection(Vec<Type>),
}

impl Type {
    pub fn width(&self) -> usize {
        match self {
            Type::Bit => 1,
            Type::Reference(_) => 64,
            Type::Collection(elements) => elements.iter().map(Type::width).sum(),
        }
    }

    pub fn is_bit(&self) -> bool {
        matches!(self, Type::Bit)
    }

    pub fn is_reference(&self) -> bool {
        matches!(self, Type::Reference(_))
    }

    pub fn elements(&self) -> Option<&[Type]> {
        match self {
            Type::Collection(elements) => Some(elements),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitVector {
    bits: Vec<bool>,
}

impl BitVector {
    pub fn new(bits: Vec<bool>) -> Self {
        Self { bits }
    }

    pub fn bits(&self) -> &[bool] {
        &self.bits
    }

    pub fn width(&self) -> usize {
        self.bits.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Slot(usize);

impl Slot {
    pub fn new(index: usize) -> Self {
        Self(index)
    }

    pub fn index(&self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intrinsic {
    Not,
    And,
    Or,
    Xor,
    Add,
    Sub,
    Mul,
    Inc,
    Dec,
    ShiftLeftOne,
    ShiftRightOne,
    Eq,
    Lt,
    IsZero,
    Select,
    DivMod,
}

impl Intrinsic {
    pub fn arity(&self) -> usize {
        match self {
            Intrinsic::Not
            | Intrinsic::Inc
            | Intrinsic::Dec
            | Intrinsic::ShiftLeftOne
            | Intrinsic::ShiftRightOne
            | Intrinsic::IsZero => 1,
            Intrinsic::And
            | Intrinsic::Or
            | Intrinsic::Xor
            | Intrinsic::Add
            | Intrinsic::Sub
            | Intrinsic::Mul
            | Intrinsic::Eq
            | Intrinsic::Lt
            | Intrinsic::DivMod => 2,
            Intrinsic::Select => 3,
        }
    }

    pub fn is_comparison(&self) -> bool {
        matches!(self, Intrinsic::Eq | Intrinsic::Lt | Intrinsic::IsZero)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Constant(BitVector),
    Nand(Box<Value>, Box<Value>),
    Intrinsic(Intrinsic, Vec<Value>),
    Collection(Vec<Value>),
    Element {
        collection: Box<Value>,
        index: usize,
    },
    Reference {
        slot: Slot,
        bit_offset: usize,
    },
    Load(Box<Value>),
    Call(Label, Vec<Value>),
    Extern(Operation, Vec<Value>),
    Branch(Box<Value>, Block, Block),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    statements: Vec<Statement>,
    result: Option<Box<Value>>,
}

impl Block {
    pub fn new(statements: Vec<Statement>, result: Option<Box<Value>>) -> Self {
        Self { statements, result }
    }

    pub fn statements(&self) -> &[Statement] {
        &self.statements
    }

    pub fn result(&self) -> Option<&Value> {
        self.result.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Statement {
    Bind { slot: Slot, value: Value },
    Store { reference: Value, value: Value },
    Evaluate { value: Value },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameter {
    slot: Slot,
}

impl Parameter {
    pub fn new(slot: Slot) -> Self {
        Self { slot }
    }

    pub fn slot(&self) -> Slot {
        self.slot
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    label: Label,
    parameters: Vec<Parameter>,
    result: Type,
    slots: Vec<Type>,
    body: Block,
}

impl Function {
    pub fn new(
        label: Label,
        parameters: Vec<Parameter>,
        result: Type,
        slots: Vec<Type>,
        body: Block,
    ) -> Self {
        Self {
            label,
            parameters,
            result,
            slots,
            body,
        }
    }

    pub fn label(&self) -> &Label {
        &self.label
    }

    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    pub fn result(&self) -> &Type {
        &self.result
    }

    pub fn slots(&self) -> &[Type] {
        &self.slots
    }

    pub fn body(&self) -> &Block {
        &self.body
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    functions: Vec<Function>,
}

impl Program {
    pub fn new(functions: Vec<Function>) -> Self {
        Self { functions }
    }

    pub fn functions(&self) -> &[Function] {
        &self.functions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_has_width_one() {
        assert_eq!(Type::Bit.width(), 1);
    }

    #[test]
    fn collection_width_is_the_sum_of_element_widths() {
        let type_ = Type::Collection(vec![
            Type::Bit,
            Type::Collection(vec![Type::Bit, Type::Bit]),
        ]);
        assert_eq!(type_.width(), 3);
    }

    #[test]
    fn reference_has_pointer_width() {
        assert_eq!(Type::Reference(Box::new(Type::Bit)).width(), 64);
    }

    #[test]
    fn bit_vector_keeps_bits() {
        let vector = BitVector::new(vec![true, false, true]);
        assert_eq!(vector.width(), 3);
        assert_eq!(vector.bits(), &[true, false, true]);
    }

    #[test]
    fn intrinsic_arity_is_fixed() {
        assert_eq!(Intrinsic::Not.arity(), 1);
        assert_eq!(Intrinsic::Add.arity(), 2);
        assert_eq!(Intrinsic::Select.arity(), 3);
        assert_eq!(Intrinsic::DivMod.arity(), 2);
    }

    #[test]
    fn only_comparisons_narrow_to_a_bit() {
        for intrinsic in [Intrinsic::Eq, Intrinsic::Lt, Intrinsic::IsZero] {
            assert!(intrinsic.is_comparison());
        }
        for intrinsic in [Intrinsic::Not, Intrinsic::Add, Intrinsic::Select] {
            assert!(!intrinsic.is_comparison());
        }
    }

    #[test]
    fn function_keeps_its_parts() {
        let function = Function::new(
            Label::new("main"),
            vec![],
            Type::Bit,
            vec![Type::Bit],
            Block::new(
                vec![],
                Some(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        assert_eq!(function.label().text(), "main");
        assert_eq!(function.slots().len(), 1);
        assert!(function.body().result().is_some());
    }

    #[test]
    fn an_empty_collection_has_width_zero() {
        assert_eq!(Type::Collection(vec![]).width(), 0);
    }

    #[test]
    fn a_nested_collection_width_is_the_sum() {
        let type_ = Type::Collection(vec![Type::Bit, Type::Collection(vec![Type::Bit; 3])]);
        assert_eq!(type_.width(), 4);
    }

    #[test]
    fn a_reference_to_a_collection_has_pointer_width() {
        let type_ = Type::Reference(Box::new(Type::Collection(vec![Type::Bit; 8])));
        assert_eq!(type_.width(), 64);
    }

    #[test]
    fn elements_are_visible_for_collections_only() {
        assert!(Type::Bit.elements().is_none());
        assert!(Type::Reference(Box::new(Type::Bit)).elements().is_none());
        assert_eq!(
            Type::Collection(vec![Type::Bit]).elements(),
            Some(&[Type::Bit][..])
        );
    }

    #[test]
    fn every_intrinsic_has_an_arity() {
        for intrinsic in [
            Intrinsic::Not,
            Intrinsic::And,
            Intrinsic::Or,
            Intrinsic::Xor,
            Intrinsic::Add,
            Intrinsic::Sub,
            Intrinsic::Mul,
            Intrinsic::Inc,
            Intrinsic::Dec,
            Intrinsic::ShiftLeftOne,
            Intrinsic::ShiftRightOne,
            Intrinsic::Eq,
            Intrinsic::Lt,
            Intrinsic::IsZero,
            Intrinsic::Select,
            Intrinsic::DivMod,
        ] {
            assert!(intrinsic.arity() >= 1);
        }
    }

    #[test]
    fn a_slot_keeps_its_index() {
        assert_eq!(Slot::new(7).index(), 7);
    }

    #[test]
    fn an_empty_bit_vector_has_width_zero() {
        assert_eq!(BitVector::new(vec![]).width(), 0);
    }

    #[test]
    fn an_empty_program_has_no_functions() {
        assert!(Program::new(vec![]).functions().is_empty());
    }
}
