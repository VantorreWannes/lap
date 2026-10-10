use lapc_ast::Label;
use lapc_extern::{Operation, OperationSpecification};

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

pub fn extern_result_type(specification: &OperationSpecification) -> Type {
    let mut elements = vec![Type::Bit];
    for width in specification.payload_widths() {
        elements.push(Type::Collection(vec![Type::Bit; *width]));
    }
    Type::Collection(elements)
}

pub fn intrinsic_for(label: &str) -> Option<Intrinsic> {
    INTRINSICS
        .iter()
        .find(|(name, _)| *name == label)
        .map(|(_, intrinsic)| *intrinsic)
}

pub fn intrinsic_signature(label: &str) -> Option<(Vec<Type>, Type)> {
    let intrinsic = intrinsic_for(label)?;
    let operand = intrinsic_operand(label)?;
    let result = if intrinsic.is_comparison() {
        Type::Bit
    } else if intrinsic == Intrinsic::DivMod {
        Type::Collection(vec![operand.clone(), operand.clone()])
    } else {
        operand.clone()
    };
    Some((vec![operand; intrinsic.arity()], result))
}

fn intrinsic_operand(label: &str) -> Option<Type> {
    let (name, _) = label.strip_prefix("liblapc.")?.split_once('.')?;
    let width = match name {
        "bit" => return Some(Type::Bit),
        "u4" => 4,
        "u8" => 8,
        "u16" => 16,
        "u32" => 32,
        "u64" => 64,
        _ => return None,
    };
    Some(Type::Collection(vec![Type::Bit; width]))
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

const INTRINSICS: &[(&str, Intrinsic)] = &[
    ("liblapc.bit.not", Intrinsic::Not),
    ("liblapc.bit.and", Intrinsic::And),
    ("liblapc.bit.or", Intrinsic::Or),
    ("liblapc.bit.xor", Intrinsic::Xor),
    ("liblapc.bit.select", Intrinsic::Select),
    ("liblapc.u4.add", Intrinsic::Add),
    ("liblapc.u4.sub", Intrinsic::Sub),
    ("liblapc.u4.mul", Intrinsic::Mul),
    ("liblapc.u4.inc", Intrinsic::Inc),
    ("liblapc.u4.dec", Intrinsic::Dec),
    ("liblapc.u4.and", Intrinsic::And),
    ("liblapc.u4.or", Intrinsic::Or),
    ("liblapc.u4.xor", Intrinsic::Xor),
    ("liblapc.u4.not", Intrinsic::Not),
    ("liblapc.u4.shift.left.one", Intrinsic::ShiftLeftOne),
    ("liblapc.u4.shift.right.one", Intrinsic::ShiftRightOne),
    ("liblapc.u4.eq", Intrinsic::Eq),
    ("liblapc.u4.lt", Intrinsic::Lt),
    ("liblapc.u4.is.zero", Intrinsic::IsZero),
    ("liblapc.u4.div.mod", Intrinsic::DivMod),
    ("liblapc.u8.add", Intrinsic::Add),
    ("liblapc.u8.sub", Intrinsic::Sub),
    ("liblapc.u8.mul", Intrinsic::Mul),
    ("liblapc.u8.inc", Intrinsic::Inc),
    ("liblapc.u8.dec", Intrinsic::Dec),
    ("liblapc.u8.and", Intrinsic::And),
    ("liblapc.u8.or", Intrinsic::Or),
    ("liblapc.u8.xor", Intrinsic::Xor),
    ("liblapc.u8.not", Intrinsic::Not),
    ("liblapc.u8.shift.left.one", Intrinsic::ShiftLeftOne),
    ("liblapc.u8.shift.right.one", Intrinsic::ShiftRightOne),
    ("liblapc.u8.eq", Intrinsic::Eq),
    ("liblapc.u8.lt", Intrinsic::Lt),
    ("liblapc.u8.is.zero", Intrinsic::IsZero),
    ("liblapc.u8.div.mod", Intrinsic::DivMod),
    ("liblapc.u16.add", Intrinsic::Add),
    ("liblapc.u16.sub", Intrinsic::Sub),
    ("liblapc.u16.mul", Intrinsic::Mul),
    ("liblapc.u16.inc", Intrinsic::Inc),
    ("liblapc.u16.dec", Intrinsic::Dec),
    ("liblapc.u16.and", Intrinsic::And),
    ("liblapc.u16.or", Intrinsic::Or),
    ("liblapc.u16.xor", Intrinsic::Xor),
    ("liblapc.u16.not", Intrinsic::Not),
    ("liblapc.u16.shift.left.one", Intrinsic::ShiftLeftOne),
    ("liblapc.u16.shift.right.one", Intrinsic::ShiftRightOne),
    ("liblapc.u16.eq", Intrinsic::Eq),
    ("liblapc.u16.lt", Intrinsic::Lt),
    ("liblapc.u16.is.zero", Intrinsic::IsZero),
    ("liblapc.u16.div.mod", Intrinsic::DivMod),
    ("liblapc.u32.add", Intrinsic::Add),
    ("liblapc.u32.sub", Intrinsic::Sub),
    ("liblapc.u32.mul", Intrinsic::Mul),
    ("liblapc.u32.inc", Intrinsic::Inc),
    ("liblapc.u32.dec", Intrinsic::Dec),
    ("liblapc.u32.and", Intrinsic::And),
    ("liblapc.u32.or", Intrinsic::Or),
    ("liblapc.u32.xor", Intrinsic::Xor),
    ("liblapc.u32.not", Intrinsic::Not),
    ("liblapc.u32.shift.left.one", Intrinsic::ShiftLeftOne),
    ("liblapc.u32.shift.right.one", Intrinsic::ShiftRightOne),
    ("liblapc.u32.eq", Intrinsic::Eq),
    ("liblapc.u32.lt", Intrinsic::Lt),
    ("liblapc.u32.is.zero", Intrinsic::IsZero),
    ("liblapc.u32.div.mod", Intrinsic::DivMod),
    ("liblapc.u64.add", Intrinsic::Add),
    ("liblapc.u64.sub", Intrinsic::Sub),
    ("liblapc.u64.mul", Intrinsic::Mul),
    ("liblapc.u64.inc", Intrinsic::Inc),
    ("liblapc.u64.dec", Intrinsic::Dec),
    ("liblapc.u64.and", Intrinsic::And),
    ("liblapc.u64.or", Intrinsic::Or),
    ("liblapc.u64.xor", Intrinsic::Xor),
    ("liblapc.u64.not", Intrinsic::Not),
    ("liblapc.u64.shift.left.one", Intrinsic::ShiftLeftOne),
    ("liblapc.u64.shift.right.one", Intrinsic::ShiftRightOne),
    ("liblapc.u64.eq", Intrinsic::Eq),
    ("liblapc.u64.lt", Intrinsic::Lt),
    ("liblapc.u64.is.zero", Intrinsic::IsZero),
    ("liblapc.u64.div.mod", Intrinsic::DivMod),
];

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

    #[test]
    fn an_extern_result_is_the_status_bit_and_the_payload_fields() {
        let specification = lapc_extern::lookup(0x0100).expect("memory.acquire is in the table");
        assert_eq!(
            extern_result_type(specification),
            Type::Collection(vec![Type::Bit, Type::Collection(vec![Type::Bit; 64])])
        );
    }

    #[test]
    fn every_table_label_is_a_lap_label() {
        for (name, _) in INTRINSICS {
            assert!(!name.is_empty());
            assert!(name.chars().all(|character| {
                character.is_ascii_alphanumeric() || character == '.' || character == '_'
            }));
        }
    }

    #[test]
    fn every_table_label_is_unique() {
        for (index, (name, _)) in INTRINSICS.iter().enumerate() {
            for (other, _) in &INTRINSICS[index + 1..] {
                assert_ne!(name, other);
            }
        }
    }

    #[test]
    fn every_table_label_has_a_signature() {
        for (name, _) in INTRINSICS {
            assert!(intrinsic_signature(name).is_some());
        }
    }

    #[test]
    fn a_binary_signature_takes_and_yields_the_operand() {
        let operand = Type::Collection(vec![Type::Bit; 8]);
        assert_eq!(
            intrinsic_signature("liblapc.u8.add"),
            Some((vec![operand.clone(), operand.clone()], operand))
        );
    }

    #[test]
    fn a_comparison_signature_yields_a_bit() {
        let operand = Type::Collection(vec![Type::Bit; 8]);
        assert_eq!(
            intrinsic_signature("liblapc.u8.lt"),
            Some((vec![operand.clone(), operand], Type::Bit))
        );
    }

    #[test]
    fn a_division_signature_yields_a_pair() {
        let operand = Type::Collection(vec![Type::Bit; 64]);
        assert_eq!(
            intrinsic_signature("liblapc.u64.div.mod"),
            Some((
                vec![operand.clone(), operand.clone()],
                Type::Collection(vec![operand.clone(), operand])
            ))
        );
    }

    #[test]
    fn a_bit_signature_takes_and_yields_a_bit() {
        assert_eq!(
            intrinsic_signature("liblapc.bit.not"),
            Some((vec![Type::Bit], Type::Bit))
        );
    }

    #[test]
    fn a_label_outside_the_table_has_no_signature() {
        assert_eq!(intrinsic_signature("liblapc.u8.identity"), None);
        assert_eq!(intrinsic_signature("liblapc.u7.add"), None);
        assert_eq!(intrinsic_signature("u8.add"), None);
    }
}
