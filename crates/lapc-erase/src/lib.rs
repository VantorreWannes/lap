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
    let operand = match function.parameters().first() {
        Some(parameter) => function.slots()[parameter.slot().index()].clone(),
        None => return function.clone(),
    };
    if operand.width() > 64 {
        return function.clone();
    }
    for parameter in function.parameters() {
        if function.slots()[parameter.slot().index()] != operand {
            return function.clone();
        }
    }
    if intrinsic.is_comparison() {
        if !result.is_bit() {
            return function.clone();
        }
    } else if result != operand {
        return function.clone();
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
            "liblapc.bit.not",
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
    fn a_comparison_is_erased_to_a_bit() {
        let function = Function::new(
            Label::new("liblapc.u8.eq"),
            vec![Parameter::new(Slot::new(0)), Parameter::new(Slot::new(1))],
            Type::Bit,
            vec![Type::Collection(vec![Type::Bit; 8]); 2],
            Block::new(
                vec![],
                Some(Box::new(Value::Constant(BitVector::new(vec![false])))),
            ),
        );
        let program = erase_intrinsics(Program::new(vec![function]));
        let erased = &program.functions()[0];
        assert!(matches!(
            erased.body().result(),
            Some(Value::Intrinsic(Intrinsic::Eq, arguments)) if arguments.len() == 2
        ));
    }

    #[test]
    fn a_comparison_with_a_wide_result_is_untouched() {
        let function = Function::new(
            Label::new("liblapc.u8.eq"),
            vec![Parameter::new(Slot::new(0)), Parameter::new(Slot::new(1))],
            Type::Collection(vec![Type::Bit; 8]),
            vec![Type::Collection(vec![Type::Bit; 8]); 2],
            Block::new(
                vec![],
                Some(Box::new(Value::Constant(BitVector::new(vec![false; 8])))),
            ),
        );
        let program = erase_intrinsics(Program::new(vec![function.clone()]));
        assert_eq!(program.functions()[0], function);
    }

    #[test]
    fn a_wide_function_is_untouched() {
        let function = Function::new(
            Label::new("liblapc.u64.add"),
            vec![Parameter::new(Slot::new(0)), Parameter::new(Slot::new(1))],
            Type::Collection(vec![Type::Bit; 128]),
            vec![Type::Collection(vec![Type::Bit; 128]); 2],
            Block::new(
                vec![],
                Some(Box::new(Value::Constant(BitVector::new(vec![false; 128])))),
            ),
        );
        let program = erase_intrinsics(Program::new(vec![function.clone()]));
        assert_eq!(program.functions()[0], function);
    }

    #[test]
    fn a_function_outside_the_table_is_untouched() {
        let function = function_with_body(
            "liblapc.bit.identity",
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
            "liblapc.bit.not",
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
            Label::new("liblapc.u8.add"),
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
            Label::new("liblapc.bit.and"),
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
            Label::new("liblapc.bit.not"),
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
            "liblapc.bit.not",
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

    #[test]
    fn every_table_label_is_unique() {
        for (index, (name, _)) in INTRINSICS.iter().enumerate() {
            for (other, _) in &INTRINSICS[index + 1..] {
                assert_ne!(name, other);
            }
        }
    }
}
