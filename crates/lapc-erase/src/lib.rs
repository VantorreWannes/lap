use lapc_ir::{Block, Function, Intrinsic, Program, Type, Value, intrinsic_for};

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
    } else if intrinsic == Intrinsic::DivMod {
        let paired = Type::Collection(vec![operand.clone(), operand.clone()]);
        if result != paired {
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
    fn a_division_is_erased_to_a_pair() {
        let function = Function::new(
            Label::new("liblapc.u64.div.mod"),
            vec![Parameter::new(Slot::new(0)), Parameter::new(Slot::new(1))],
            Type::Collection(vec![
                Type::Collection(vec![Type::Bit; 64]),
                Type::Collection(vec![Type::Bit; 64]),
            ]),
            vec![Type::Collection(vec![Type::Bit; 64]); 2],
            Block::new(
                vec![],
                Some(Box::new(Value::Constant(BitVector::new(vec![false; 128])))),
            ),
        );
        let program = erase_intrinsics(Program::new(vec![function]));
        let erased = &program.functions()[0];
        assert!(matches!(
            erased.body().result(),
            Some(Value::Intrinsic(Intrinsic::DivMod, arguments)) if arguments.len() == 2
        ));
    }

    #[test]
    fn a_division_with_an_unpaired_result_is_untouched() {
        let function = Function::new(
            Label::new("liblapc.u64.div.mod"),
            vec![Parameter::new(Slot::new(0)), Parameter::new(Slot::new(1))],
            Type::Collection(vec![Type::Bit; 64]),
            vec![Type::Collection(vec![Type::Bit; 64]); 2],
            Block::new(
                vec![],
                Some(Box::new(Value::Constant(BitVector::new(vec![false; 64])))),
            ),
        );
        let program = erase_intrinsics(Program::new(vec![function.clone()]));
        assert_eq!(program.functions()[0], function);
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

}
