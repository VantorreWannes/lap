use lapc_ir::{Block, Function, Intrinsic, Program, Value};

pub fn erase_intrinsics(program: Program) -> Program {
    let functions = program.functions().iter().map(erase_function).collect();
    Program::new(functions)
}

fn erase_function(function: &Function) -> Function {
    let intrinsic = match intrinsic_for(function.label().text()) {
        Some(intrinsic) => intrinsic,
        None => return function.clone(),
    };
    if function.parameters().len() != intrinsic.arity() {
        return function.clone();
    }
    let result = function.result().clone();
    if result.is_reference() {
        return function.clone();
    }
    for parameter in function.parameters() {
        let type_ = function.slots()[parameter.slot().index()].clone();
        if type_ != result {
            return function.clone();
        }
    }
    let arguments = function
        .parameters()
        .iter()
        .map(|parameter| {
            Value::Load(Box::new(Value::Reference {
                slot: parameter.slot(),
                bit_offset: 0,
            }))
        })
        .collect();
    let body = Block::new(
        vec![],
        Some(Box::new(Value::Intrinsic(intrinsic, arguments))),
    );
    Function::new(
        function.label().clone(),
        function.parameters().to_vec(),
        result,
        function.slots().to_vec(),
        body,
    )
}

fn intrinsic_for(label: &str) -> Option<Intrinsic> {
    INTRINSICS
        .iter()
        .find(|(name, _)| *name == label)
        .map(|(_, intrinsic)| *intrinsic)
}

const INTRINSICS: &[(&str, Intrinsic)] = &[
    ("bit.not", Intrinsic::Not),
    ("bit.and", Intrinsic::And),
    ("bit.or", Intrinsic::Or),
    ("bit.xor", Intrinsic::Xor),
    ("u8.add", Intrinsic::Add),
    ("u8.sub", Intrinsic::Sub),
];

#[cfg(test)]
mod tests {
    use super::*;
    use lapc_ast::Label;
    use lapc_ir::{BitVector, Parameter, Slot, Statement, Type};

    fn function_with_body(label: &str, parameters: usize, type_: Type, body: Value) -> Function {
        let slots = vec![type_.clone(); parameters];
        let parameters = (0..parameters)
            .map(|index| Parameter::new(Slot::new(index)))
            .collect();
        Function::new(
            Label::new(label),
            parameters,
            type_,
            slots,
            Block::new(vec![], Some(Box::new(body))),
        )
    }

    #[test]
    fn a_matching_function_is_erased() {
        let function = function_with_body(
            "bit.not",
            1,
            Type::Bit,
            Value::Nand(
                Box::new(Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                }))),
                Box::new(Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                }))),
            ),
        );
        let program = erase_intrinsics(Program::new(vec![function]));
        let erased = &program.functions()[0];
        assert_eq!(
            erased.body().result(),
            Some(&Value::Intrinsic(
                Intrinsic::Not,
                vec![Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0
                }))],
            )),
        );
    }

    #[test]
    fn a_function_outside_the_table_is_untouched() {
        let function = function_with_body(
            "bit.identity",
            1,
            Type::Bit,
            Value::Load(Box::new(Value::Reference {
                slot: Slot::new(0),
                bit_offset: 0,
            })),
        );
        let program = erase_intrinsics(Program::new(vec![function.clone()]));
        assert_eq!(program.functions()[0], function);
    }

    #[test]
    fn a_function_with_the_wrong_arity_is_untouched() {
        let function = function_with_body(
            "bit.not",
            2,
            Type::Bit,
            Value::Constant(BitVector::new(vec![false])),
        );
        let program = erase_intrinsics(Program::new(vec![function.clone()]));
        assert_eq!(program.functions()[0], function);
    }

    #[test]
    fn a_function_with_the_wrong_width_is_untouched() {
        let function = Function::new(
            Label::new("u8.add"),
            vec![Parameter::new(Slot::new(0)), Parameter::new(Slot::new(1))],
            Type::Collection(vec![Type::Bit, Type::Bit, Type::Bit]),
            vec![
                Type::Collection(vec![Type::Bit, Type::Bit]),
                Type::Collection(vec![Type::Bit, Type::Bit]),
            ],
            Block::new(
                vec![],
                Some(Box::new(Value::Constant(BitVector::new(vec![
                    false, false, false,
                ])))),
            ),
        );
        let program = erase_intrinsics(Program::new(vec![function.clone()]));
        assert_eq!(program.functions()[0], function);
    }

    #[test]
    fn a_statement_body_is_replaced_by_the_intrinsic() {
        let function = Function::new(
            Label::new("bit.and"),
            vec![Parameter::new(Slot::new(0)), Parameter::new(Slot::new(1))],
            Type::Bit,
            vec![Type::Bit, Type::Bit],
            Block::new(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: Value::Constant(BitVector::new(vec![false])),
                }],
                Some(Box::new(Value::Constant(BitVector::new(vec![true])))),
            ),
        );
        let program = erase_intrinsics(Program::new(vec![function]));
        let erased = &program.functions()[0];
        assert!(erased.body().statements().is_empty());
        assert_eq!(
            erased.body().result(),
            Some(&Value::Intrinsic(
                Intrinsic::And,
                vec![
                    Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: 0
                    })),
                    Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(1),
                        bit_offset: 0
                    })),
                ],
            )),
        );
    }

    #[test]
    fn a_reference_result_is_untouched() {
        let function = Function::new(
            Label::new("bit.not"),
            vec![Parameter::new(Slot::new(0))],
            Type::Reference(Box::new(Type::Bit)),
            vec![Type::Reference(Box::new(Type::Bit))],
            Block::new(
                vec![],
                Some(Box::new(Value::Constant(BitVector::new(vec![false])))),
            ),
        );
        let program = erase_intrinsics(Program::new(vec![function.clone()]));
        assert_eq!(program.functions()[0], function);
    }

    #[test]
    fn erasing_twice_is_erasing_once() {
        let function = function_with_body(
            "bit.not",
            1,
            Type::Bit,
            Value::Constant(BitVector::new(vec![false])),
        );
        let once = erase_intrinsics(Program::new(vec![function]));
        let twice = erase_intrinsics(once.clone());
        assert_eq!(once, twice);
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
}
