use std::collections::{BTreeSet, HashMap};

use lapc_ast::{
    Binding, BindingValue, Block, Expression, FunctionDefinition, Label, NamedTarget, Program,
    Statement, Target, TypeExpression,
};
use lapc_extern::lookup;
use lapc_ir::{
    BitVector, Block as IrBlock, Function, Parameter as IrParameter, Program as IrProgram, Slot,
    Statement as IrStatement, Type, Value, extern_result_type, intrinsic_signature,
};

pub fn check_program(program: &Program) -> Result<IrProgram, CheckError> {
    let mut checker = Checker::new();
    for statement in program.statements() {
        checker.check_top_level(statement)?;
    }
    checker.check_entry()?;
    Ok(IrProgram::new(checker.functions))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckError {
    kind: CheckErrorKind,
    label: Option<Label>,
}

impl CheckError {
    fn new(kind: CheckErrorKind) -> Self {
        Self { kind, label: None }
    }

    fn at(kind: CheckErrorKind, label: &Label) -> Self {
        Self {
            kind,
            label: Some(label.clone()),
        }
    }

    pub fn kind(&self) -> &CheckErrorKind {
        &self.kind
    }

    pub fn label(&self) -> Option<&Label> {
        self.label.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckErrorKind {
    UnknownLabel,
    ShadowedGlobal,
    ExpectedType,
    ExpectedValue,
    ExpectedFunction,
    TypeMismatch,
    Indeterminate,
    UnknownOperation,
    WrongArgumentCount,
    WidthMismatch,
    ReferenceInCollection,
    ReferenceResult,
    ReferenceArgument,
    MissingEntry,
    EntrySignature,
    NotAConstant,
    NotDestructurable,
    WrongElementCount,
    TopLevelExpression,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BitDeterminacy {
    Zero,
    One,
    Bit,
    NotBit,
    Unknown,
    Determinate,
    Guarded(BTreeSet<usize>),
}

impl BitDeterminacy {
    fn from_bit(bit: bool) -> Self {
        if bit { Self::One } else { Self::Zero }
    }

    fn nand(left: &Self, right: &Self) -> Self {
        match (left, right) {
            (Self::Zero, _) | (_, Self::Zero) => Self::One,
            (Self::One, Self::One) => Self::Zero,
            (Self::One, Self::Bit) | (Self::Bit, Self::One) => Self::NotBit,
            (Self::One, Self::NotBit) | (Self::NotBit, Self::One) => Self::Bit,
            (Self::Bit, Self::Bit) => Self::NotBit,
            (Self::Bit, Self::NotBit) | (Self::NotBit, Self::Bit) => Self::One,
            (Self::NotBit, Self::NotBit) => Self::Bit,
            (Self::One, Self::Determinate) | (Self::Determinate, Self::One) => Self::Determinate,
            (Self::Determinate, Self::Determinate) => Self::Determinate,
            (Self::One, Self::Guarded(set)) | (Self::Guarded(set), Self::One) => {
                Self::Guarded(set.clone())
            }
            (Self::Determinate, Self::Guarded(set)) | (Self::Guarded(set), Self::Determinate) => {
                Self::Guarded(set.clone())
            }
            (Self::Guarded(left), Self::Guarded(right)) => {
                Self::Guarded(left.union(right).copied().collect())
            }
            _ => Self::Unknown,
        }
    }

    fn is_determinate(&self, guards: &[usize]) -> bool {
        match self {
            Self::Zero | Self::One | Self::Determinate => true,
            Self::Guarded(set) => set.iter().all(|call| guards.contains(call)),
            _ => false,
        }
    }

    fn guard_requirements(&self) -> Option<BTreeSet<usize>> {
        match self {
            Self::Zero | Self::One | Self::Determinate => Some(BTreeSet::new()),
            Self::Guarded(set) => Some(set.clone()),
            _ => None,
        }
    }
}

struct CheckedValue {
    type_: Type,
    bits: Vec<BitDeterminacy>,
    extern_call: Option<usize>,
    reference_target: Option<ReferenceTarget>,
    value: Value,
}

impl CheckedValue {
    fn is_determinate(&self, guards: &[usize]) -> bool {
        self.bits.iter().all(|bit| bit.is_determinate(guards))
    }

    fn constant_bits(&self) -> Option<Vec<bool>> {
        self.bits
            .iter()
            .map(|bit| match bit {
                BitDeterminacy::Zero => Some(false),
                BitDeterminacy::One => Some(true),
                _ => None,
            })
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ReferenceTarget {
    slot: Slot,
    bit_offset: usize,
}

#[derive(Clone)]
struct LocalBinding {
    slot: Slot,
    type_: Type,
    extern_call: Option<usize>,
    reference_target: Option<ReferenceTarget>,
}

enum GlobalBinding {
    Type(Type),
    Constant { type_: Type, bits: Vec<bool> },
    Function { parameters: Vec<Type>, result: Type },
}

struct FunctionBuilder {
    slots: Vec<Type>,
    statements: Vec<IrStatement>,
}

impl FunctionBuilder {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            statements: Vec::new(),
        }
    }

    fn add_slot(&mut self, type_: Type) -> Slot {
        let slot = Slot::new(self.slots.len());
        self.slots.push(type_);
        slot
    }
}

struct Checker {
    globals: HashMap<Label, GlobalBinding>,
    functions: Vec<Function>,
    builder: FunctionBuilder,
    scope: HashMap<Label, LocalBinding>,
    slot_bits: Vec<Vec<BitDeterminacy>>,
    guards: Vec<usize>,
    extern_count: usize,
}

impl Checker {
    fn new() -> Self {
        Self {
            globals: HashMap::new(),
            functions: Vec::new(),
            builder: FunctionBuilder::new(),
            scope: HashMap::new(),
            slot_bits: Vec::new(),
            guards: Vec::new(),
            extern_count: 0,
        }
    }

    fn add_slot(&mut self, type_: Type) -> Slot {
        let slot = self.builder.add_slot(type_);
        self.slot_bits.push(Vec::new());
        slot
    }

    fn set_slot_bits(&mut self, slot: Slot, bits: Vec<BitDeterminacy>) {
        self.slot_bits[slot.index()] = bits;
    }

    fn check_top_level(&mut self, statement: &Statement) -> Result<(), CheckError> {
        match statement {
            Statement::Binding(binding) => self.check_top_level_binding(binding),
            Statement::Expression(_) => Err(CheckError::new(CheckErrorKind::TopLevelExpression)),
        }
    }

    fn check_top_level_binding(&mut self, binding: &Binding) -> Result<(), CheckError> {
        let label = match binding.target() {
            Target::Named(named) => named.label().clone(),
            _ => return Err(CheckError::new(CheckErrorKind::ExpectedValue)),
        };
        if self.globals.contains_key(&label) {
            return Err(CheckError::at(CheckErrorKind::ShadowedGlobal, &label));
        }
        match binding.value() {
            BindingValue::Function(definition) => self.check_function(&label, definition),
            BindingValue::Expression(expression) => {
                if let Expression::Collection(elements) = expression {
                    if elements.is_empty() {
                        if let Some((parameters, result)) = intrinsic_signature(label.text()) {
                            return self.check_erased_function(&label, parameters, result);
                        }
                    }
                }
                if let Some(type_expression) = self.as_type_expression(expression) {
                    let type_ = self.resolve_type(&type_expression)?;
                    self.globals.insert(label, GlobalBinding::Type(type_));
                    return Ok(());
                }
                let value = self.check_expression(expression)?;
                let bits = value
                    .constant_bits()
                    .ok_or_else(|| CheckError::at(CheckErrorKind::NotAConstant, &label))?;
                self.globals.insert(
                    label,
                    GlobalBinding::Constant {
                        type_: value.type_,
                        bits,
                    },
                );
                Ok(())
            }
        }
    }

    fn check_erased_function(
        &mut self,
        label: &Label,
        parameters: Vec<Type>,
        result: Type,
    ) -> Result<(), CheckError> {
        self.globals.insert(
            label.clone(),
            GlobalBinding::Function {
                parameters: parameters.clone(),
                result: result.clone(),
            },
        );
        let ir_parameters = (0..parameters.len())
            .map(|index| IrParameter::new(Slot::new(index)))
            .collect();
        let body = IrBlock::new(
            vec![],
            Some(Box::new(Value::Constant(BitVector::new(vec![
                false;
                result
                    .width()
            ])))),
        );
        self.functions.push(Function::new(
            label.clone(),
            ir_parameters,
            result,
            parameters,
            body,
        ));
        Ok(())
    }

    fn check_function(
        &mut self,
        label: &Label,
        definition: &FunctionDefinition,
    ) -> Result<(), CheckError> {
        let mut parameters = Vec::new();
        for parameter in definition.parameters() {
            parameters.push(self.resolve_type(parameter.type_expression())?);
        }
        let result = self.resolve_type(definition.result())?;
        if result.is_reference() {
            return Err(CheckError::at(CheckErrorKind::ReferenceResult, label));
        }
        self.globals.insert(
            label.clone(),
            GlobalBinding::Function {
                parameters: parameters.clone(),
                result: result.clone(),
            },
        );
        let saved_builder = std::mem::replace(&mut self.builder, FunctionBuilder::new());
        let saved_scope = std::mem::take(&mut self.scope);
        let saved_slot_bits = std::mem::take(&mut self.slot_bits);
        let saved_guards = std::mem::take(&mut self.guards);
        let mut ir_parameters = Vec::new();
        for (parameter, type_) in definition.parameters().iter().zip(parameters.iter()) {
            if self.globals.contains_key(parameter.label()) {
                return Err(CheckError::at(
                    CheckErrorKind::ShadowedGlobal,
                    parameter.label(),
                ));
            }
            let slot = self.add_slot(type_.clone());
            self.set_slot_bits(slot, vec![BitDeterminacy::Determinate; type_.width()]);
            self.scope.insert(
                parameter.label().clone(),
                LocalBinding {
                    slot,
                    type_: type_.clone(),
                    extern_call: None,
                    reference_target: None,
                },
            );
            ir_parameters.push(IrParameter::new(slot));
        }
        let body = self.check_block(definition.body())?;
        let ir_body = match body {
            Some(value) => {
                if value.type_ != result {
                    return Err(CheckError::at(CheckErrorKind::TypeMismatch, label));
                }
                if !value.is_determinate(&self.guards) {
                    return Err(CheckError::at(CheckErrorKind::Indeterminate, label));
                }
                IrBlock::new(
                    std::mem::take(&mut self.builder.statements),
                    Some(Box::new(value.value)),
                )
            }
            None => {
                if result.width() != 0 {
                    return Err(CheckError::at(CheckErrorKind::TypeMismatch, label));
                }
                IrBlock::new(std::mem::take(&mut self.builder.statements), None)
            }
        };
        let slots = std::mem::take(&mut self.builder.slots);
        self.builder = saved_builder;
        self.scope = saved_scope;
        self.slot_bits = saved_slot_bits;
        self.guards = saved_guards;
        self.functions.push(Function::new(
            label.clone(),
            ir_parameters,
            result,
            slots,
            ir_body,
        ));
        Ok(())
    }

    fn check_entry(&self) -> Result<(), CheckError> {
        let entry = Label::new("main");
        match self.globals.get(&entry) {
            Some(GlobalBinding::Function { parameters, result }) => {
                if !parameters.is_empty() || !result.is_bit() {
                    return Err(CheckError::at(CheckErrorKind::EntrySignature, &entry));
                }
                Ok(())
            }
            _ => Err(CheckError::at(CheckErrorKind::MissingEntry, &entry)),
        }
    }

    fn check_block(&mut self, block: &Block) -> Result<Option<CheckedValue>, CheckError> {
        let saved_scope = self.scope.clone();
        let saved_slot_bits = self.slot_bits.clone();
        let mut result = None;
        for statement in block.statements() {
            match statement {
                Statement::Binding(binding) => self.check_binding(binding)?,
                Statement::Expression(expression) => {
                    let checked = self.check_expression(expression)?;
                    self.builder.statements.push(IrStatement::Evaluate {
                        value: checked.value,
                    });
                }
            }
        }
        if let Some(expression) = block.result() {
            result = Some(self.check_expression(expression)?);
        }
        self.scope = saved_scope;
        self.slot_bits = saved_slot_bits;
        self.slot_bits.resize(self.builder.slots.len(), Vec::new());
        Ok(result)
    }

    fn check_binding(&mut self, binding: &Binding) -> Result<(), CheckError> {
        match binding.target() {
            Target::Named(named) => self.check_named_binding(named, binding.value()),
            Target::Reference(label) => self.check_reference_binding(label, binding.value()),
            Target::Destructuring(targets) => self.check_destructuring(targets, binding.value()),
        }
    }

    fn check_named_binding(
        &mut self,
        named: &NamedTarget,
        value: &BindingValue,
    ) -> Result<(), CheckError> {
        let label = named.label();
        let expression = match value {
            BindingValue::Expression(expression) => expression,
            BindingValue::Function(_) => {
                return Err(CheckError::at(CheckErrorKind::ExpectedValue, label));
            }
        };
        if let Some(local) = self.scope.get(label)
            && let Type::Reference(pointed) = local.type_.clone()
        {
            let slot = local.slot;
            let target = local.reference_target;
            let pointed = *pointed;
            self.check_annotation(named, &pointed, label)?;
            return self.store_through_reference(slot, target, &pointed, expression, label);
        }
        if self.globals.contains_key(label) {
            return Err(CheckError::at(CheckErrorKind::ShadowedGlobal, label));
        }
        let checked = self.check_expression(expression)?;
        self.check_annotation(named, &checked.type_, label)?;
        self.bind_local_value(label, checked);
        Ok(())
    }

    fn check_annotation(
        &self,
        named: &NamedTarget,
        expected: &Type,
        label: &Label,
    ) -> Result<(), CheckError> {
        if let Some(type_expression) = named.type_expression() {
            let annotated = self.resolve_type(type_expression)?;
            if annotated != *expected {
                return Err(CheckError::at(CheckErrorKind::TypeMismatch, label));
            }
        }
        Ok(())
    }

    fn check_reference_binding(
        &mut self,
        label: &Label,
        value: &BindingValue,
    ) -> Result<(), CheckError> {
        let expression = match value {
            BindingValue::Expression(expression) => expression,
            BindingValue::Function(_) => {
                return Err(CheckError::at(CheckErrorKind::ExpectedValue, label));
            }
        };
        let (slot, target, pointed) = {
            let local = self.lookup_local(label)?;
            match &local.type_ {
                Type::Reference(pointed) => {
                    (local.slot, local.reference_target, (**pointed).clone())
                }
                other => (
                    local.slot,
                    Some(ReferenceTarget {
                        slot: local.slot,
                        bit_offset: 0,
                    }),
                    other.clone(),
                ),
            }
        };
        self.store_through_reference(slot, target, &pointed, expression, label)
    }

    fn store_through_reference(
        &mut self,
        slot: Slot,
        target: Option<ReferenceTarget>,
        pointed: &Type,
        expression: &Expression,
        label: &Label,
    ) -> Result<(), CheckError> {
        let checked = self.check_expression(expression)?;
        if checked.type_ != *pointed {
            return Err(CheckError::at(CheckErrorKind::TypeMismatch, label));
        }
        if !checked.is_determinate(&self.guards) {
            return Err(CheckError::at(CheckErrorKind::Indeterminate, label));
        }
        if let Some(target) = target {
            self.update_target_bits(target, &checked.bits);
        }
        self.builder.statements.push(IrStatement::Store {
            reference: Value::Reference {
                slot,
                bit_offset: 0,
            },
            value: checked.value,
        });
        Ok(())
    }

    fn bind_local_value(&mut self, label: &Label, checked: CheckedValue) {
        let slot = self.add_slot(checked.type_.clone());
        self.set_slot_bits(slot, checked.bits);
        self.builder.statements.push(IrStatement::Bind {
            slot,
            value: checked.value,
        });
        self.scope.insert(
            label.clone(),
            LocalBinding {
                slot,
                type_: checked.type_,
                extern_call: checked.extern_call,
                reference_target: checked.reference_target,
            },
        );
    }

    fn check_destructuring(
        &mut self,
        targets: &[Target],
        value: &BindingValue,
    ) -> Result<(), CheckError> {
        let expression = match value {
            BindingValue::Expression(expression) => expression,
            BindingValue::Function(_) => {
                return Err(CheckError::new(CheckErrorKind::ExpectedValue));
            }
        };
        let checked = self.check_expression(expression)?;
        let elements = match &checked.type_ {
            Type::Collection(elements) => elements.clone(),
            _ => return Err(CheckError::new(CheckErrorKind::NotDestructurable)),
        };
        if elements.len() != targets.len() {
            return Err(CheckError::new(CheckErrorKind::WrongElementCount));
        }
        let needs_slot = source_needs_slot(targets, &checked.value);
        let (value, slot) = if needs_slot {
            let slot = self.add_slot(checked.type_.clone());
            self.builder.statements.push(IrStatement::Bind {
                slot,
                value: checked.value,
            });
            (
                Value::Load(Box::new(Value::Reference {
                    slot,
                    bit_offset: 0,
                })),
                Some(slot),
            )
        } else {
            (checked.value, None)
        };
        let source = DestructuringSource {
            value,
            slot,
            bits: checked.bits,
            type_: checked.type_,
            bit_offset: 0,
        };
        for (index, target) in targets.iter().enumerate() {
            let extern_call = if index == 0 {
                checked.extern_call
            } else {
                None
            };
            self.bind_destructuring_target(target, &source, index, extern_call)?;
        }
        Ok(())
    }

    fn bind_destructuring_target(
        &mut self,
        target: &Target,
        source: &DestructuringSource,
        index: usize,
        extern_call: Option<usize>,
    ) -> Result<(), CheckError> {
        let element_type = match source.type_.elements() {
            Some(elements) => elements
                .get(index)
                .cloned()
                .ok_or_else(|| CheckError::new(CheckErrorKind::WrongElementCount))?,
            None => return Err(CheckError::new(CheckErrorKind::NotDestructurable)),
        };
        let element_offset = element_bit_offset(&source.type_, index)?;
        let element_bits =
            source.bits[element_offset..element_offset + element_type.width()].to_vec();
        let element = DestructuringSource {
            value: Value::Element {
                collection: Box::new(source.value.clone()),
                index,
            },
            slot: source.slot,
            bits: element_bits,
            type_: element_type,
            bit_offset: source.bit_offset + element_offset,
        };
        match target {
            Target::Named(named) => {
                let label = named.label();
                if self.globals.contains_key(label) {
                    return Err(CheckError::at(CheckErrorKind::ShadowedGlobal, label));
                }
                self.check_annotation(named, &element.type_, label)?;
                let checked = CheckedValue {
                    type_: element.type_.clone(),
                    bits: element.bits,
                    extern_call,
                    reference_target: None,
                    value: element.value,
                };
                self.bind_local_value(label, checked);
                Ok(())
            }
            Target::Reference(label) => {
                let slot = source
                    .slot
                    .ok_or_else(|| CheckError::at(CheckErrorKind::TypeMismatch, label))?;
                let reference = Value::Reference {
                    slot,
                    bit_offset: element.bit_offset,
                };
                let reference_slot =
                    self.add_slot(Type::Reference(Box::new(element.type_.clone())));
                self.builder.statements.push(IrStatement::Bind {
                    slot: reference_slot,
                    value: reference,
                });
                self.scope.insert(
                    label.clone(),
                    LocalBinding {
                        slot: reference_slot,
                        type_: Type::Reference(Box::new(element.type_.clone())),
                        extern_call: None,
                        reference_target: Some(ReferenceTarget {
                            slot,
                            bit_offset: element.bit_offset,
                        }),
                    },
                );
                Ok(())
            }
            Target::Destructuring(targets) => {
                let elements = match element.type_.elements() {
                    Some(elements) => elements,
                    None => return Err(CheckError::new(CheckErrorKind::NotDestructurable)),
                };
                if elements.len() != targets.len() {
                    return Err(CheckError::new(CheckErrorKind::WrongElementCount));
                }
                for (nested_index, nested_target) in targets.iter().enumerate() {
                    self.bind_destructuring_target(nested_target, &element, nested_index, None)?;
                }
                Ok(())
            }
        }
    }

    fn check_expression(&mut self, expression: &Expression) -> Result<CheckedValue, CheckError> {
        match expression {
            Expression::Bit => Ok(CheckedValue {
                type_: Type::Bit,
                bits: vec![BitDeterminacy::Bit],
                extern_call: None,
                reference_target: None,
                value: Value::Constant(BitVector::new(vec![false])),
            }),
            Expression::Label(label) => self.check_label(label),
            Expression::Reference(label) => self.check_reference(label),
            Expression::Nand(left, right) => self.check_nand(left, right),
            Expression::Branch(condition, then_block, else_block) => {
                self.check_branch(condition, then_block, else_block)
            }
            Expression::Extern(arguments) => self.check_extern(arguments),
            Expression::Collection(elements) => self.check_collection(elements),
            Expression::Call(label, arguments) => self.check_call(label, arguments),
        }
    }

    fn check_label(&mut self, label: &Label) -> Result<CheckedValue, CheckError> {
        if let Some(local) = self.scope.get(label) {
            let slot = local.slot;
            let type_ = local.type_.clone();
            let extern_call = local.extern_call;
            let reference_target = local.reference_target;
            if let Type::Reference(pointed) = type_ {
                let bits = match reference_target {
                    Some(target) => self.target_bits(target, pointed.width()),
                    None => vec![BitDeterminacy::Determinate; pointed.width()],
                };
                return Ok(CheckedValue {
                    type_: *pointed,
                    bits,
                    extern_call: None,
                    reference_target,
                    value: Value::Load(Box::new(Value::Reference {
                        slot,
                        bit_offset: 0,
                    })),
                });
            }
            let bits = self.slot_bits[slot.index()].clone();
            return Ok(CheckedValue {
                type_,
                bits,
                extern_call,
                reference_target: None,
                value: Value::Load(Box::new(Value::Reference {
                    slot,
                    bit_offset: 0,
                })),
            });
        }
        match self.globals.get(label) {
            Some(GlobalBinding::Constant { type_, bits }) => Ok(CheckedValue {
                type_: type_.clone(),
                bits: bits
                    .iter()
                    .map(|bit| BitDeterminacy::from_bit(*bit))
                    .collect(),
                extern_call: None,
                reference_target: None,
                value: Value::Constant(BitVector::new(bits.clone())),
            }),
            Some(GlobalBinding::Type(_)) => {
                Err(CheckError::at(CheckErrorKind::ExpectedValue, label))
            }
            Some(GlobalBinding::Function { .. }) => {
                Err(CheckError::at(CheckErrorKind::ExpectedValue, label))
            }
            None => Err(CheckError::at(CheckErrorKind::UnknownLabel, label)),
        }
    }

    fn check_reference(&mut self, label: &Label) -> Result<CheckedValue, CheckError> {
        let local = self.lookup_local(label)?;
        let (slot, target, pointed) = match &local.type_ {
            Type::Reference(pointed) => (local.slot, local.reference_target, (**pointed).clone()),
            other => (
                local.slot,
                Some(ReferenceTarget {
                    slot: local.slot,
                    bit_offset: 0,
                }),
                other.clone(),
            ),
        };
        let bits = match target {
            Some(target) => self.target_bits(target, pointed.width()),
            None => vec![BitDeterminacy::Determinate; pointed.width()],
        };
        Ok(CheckedValue {
            type_: Type::Reference(Box::new(pointed)),
            bits,
            extern_call: None,
            reference_target: target,
            value: Value::Reference {
                slot,
                bit_offset: 0,
            },
        })
    }

    fn check_nand(
        &mut self,
        left: &Expression,
        right: &Expression,
    ) -> Result<CheckedValue, CheckError> {
        let left = self.check_expression(left)?;
        let right = self.check_expression(right)?;
        if !left.type_.is_bit() || !right.type_.is_bit() {
            return Err(CheckError::new(CheckErrorKind::TypeMismatch));
        }
        Ok(CheckedValue {
            type_: Type::Bit,
            bits: vec![BitDeterminacy::nand(&left.bits[0], &right.bits[0])],
            extern_call: None,
            reference_target: None,
            value: Value::Nand(Box::new(left.value), Box::new(right.value)),
        })
    }

    fn check_branch(
        &mut self,
        condition: &Expression,
        then_block: &Block,
        else_block: &Block,
    ) -> Result<CheckedValue, CheckError> {
        let condition = self.check_expression(condition)?;
        if !condition.type_.is_bit() {
            return Err(CheckError::new(CheckErrorKind::TypeMismatch));
        }
        if !condition.is_determinate(&self.guards) {
            return Err(CheckError::new(CheckErrorKind::Indeterminate));
        }
        let saved_statements = self.builder.statements.len();
        if let Some(call) = condition.extern_call {
            self.guards.push(call);
        }
        let then_value = self.check_block(then_block)?.unwrap_or_else(empty_value);
        if condition.extern_call.is_some() {
            self.guards.pop();
        }
        let then_statements = self.builder.statements.split_off(saved_statements);
        let else_value = self.check_block(else_block)?.unwrap_or_else(empty_value);
        let else_statements = self.builder.statements.split_off(saved_statements);
        if then_value.type_ != else_value.type_ {
            return Err(CheckError::new(CheckErrorKind::TypeMismatch));
        }
        let then_bits = discharge_guard(&then_value.bits, condition.extern_call);
        let bits = match condition.bits[0] {
            BitDeterminacy::Zero => else_value.bits.clone(),
            BitDeterminacy::One => then_bits,
            BitDeterminacy::Determinate => merge_determinacy(&then_bits, &else_value.bits),
            _ => {
                if then_bits == else_value.bits {
                    then_bits
                } else {
                    vec![BitDeterminacy::Unknown; then_value.bits.len()]
                }
            }
        };
        Ok(CheckedValue {
            type_: then_value.type_,
            bits,
            extern_call: None,
            reference_target: if then_value.reference_target == else_value.reference_target {
                then_value.reference_target
            } else {
                None
            },
            value: Value::Branch(
                Box::new(condition.value),
                IrBlock::new(then_statements, Some(Box::new(then_value.value))),
                IrBlock::new(else_statements, Some(Box::new(else_value.value))),
            ),
        })
    }

    fn check_extern(&mut self, arguments: &[Expression]) -> Result<CheckedValue, CheckError> {
        let (operation_expression, argument_expressions) = arguments
            .split_first()
            .ok_or_else(|| CheckError::new(CheckErrorKind::WrongArgumentCount))?;
        let operation = self.check_expression(operation_expression)?;
        let operation_bits = operation
            .constant_bits()
            .ok_or_else(|| CheckError::new(CheckErrorKind::NotAConstant))?;
        if operation_bits.len() != 16 {
            return Err(CheckError::new(CheckErrorKind::WidthMismatch));
        }
        let code = bits_to_u16(&operation_bits);
        let specification =
            lookup(code).ok_or_else(|| CheckError::new(CheckErrorKind::UnknownOperation))?;
        if specification.argument_widths().len() != argument_expressions.len() {
            return Err(CheckError::new(CheckErrorKind::WrongArgumentCount));
        }
        let mut argument_values = Vec::new();
        for (expression, width) in argument_expressions
            .iter()
            .zip(specification.argument_widths())
        {
            let argument = self.check_expression(expression)?;
            if argument.type_.is_reference() {
                return Err(CheckError::new(CheckErrorKind::ReferenceArgument));
            }
            if argument.type_.width() != *width {
                return Err(CheckError::new(CheckErrorKind::WidthMismatch));
            }
            if !argument.is_determinate(&self.guards) {
                return Err(CheckError::new(CheckErrorKind::Indeterminate));
            }
            argument_values.push(argument.value);
        }
        let call = self.extern_count;
        self.extern_count += 1;
        let mut bits = vec![BitDeterminacy::Determinate];
        for width in specification.payload_widths() {
            let payload = BitDeterminacy::Guarded(BTreeSet::from([call]));
            bits.extend(std::iter::repeat_n(payload, *width));
        }
        Ok(CheckedValue {
            type_: extern_result_type(specification),
            bits,
            extern_call: Some(call),
            reference_target: None,
            value: Value::Extern(specification.operation(), argument_values),
        })
    }

    fn check_collection(&mut self, elements: &[Expression]) -> Result<CheckedValue, CheckError> {
        let mut types = Vec::new();
        let mut bits = Vec::new();
        let mut values = Vec::new();
        for element in elements {
            let checked = self.check_expression(element)?;
            if checked.type_.is_reference() {
                return Err(CheckError::new(CheckErrorKind::ReferenceInCollection));
            }
            types.push(checked.type_);
            bits.extend(checked.bits);
            values.push(checked.value);
        }
        Ok(CheckedValue {
            type_: Type::Collection(types),
            bits,
            extern_call: None,
            reference_target: None,
            value: Value::Collection(values),
        })
    }

    fn check_call(
        &mut self,
        label: &Label,
        arguments: &[Expression],
    ) -> Result<CheckedValue, CheckError> {
        let (parameters, result) = match self.globals.get(label) {
            Some(GlobalBinding::Function { parameters, result }) => {
                (parameters.clone(), result.clone())
            }
            Some(_) => return Err(CheckError::at(CheckErrorKind::ExpectedFunction, label)),
            None => return Err(CheckError::at(CheckErrorKind::UnknownLabel, label)),
        };
        if parameters.len() != arguments.len() {
            return Err(CheckError::at(CheckErrorKind::WrongArgumentCount, label));
        }
        let mut argument_values = Vec::new();
        for (expression, parameter) in arguments.iter().zip(parameters.iter()) {
            let argument = self.check_expression(expression)?;
            if argument.type_ != *parameter {
                return Err(CheckError::at(CheckErrorKind::TypeMismatch, label));
            }
            if !argument.is_determinate(&self.guards) {
                return Err(CheckError::at(CheckErrorKind::Indeterminate, label));
            }
            if let Some(target) = argument.reference_target {
                let width = argument.bits.len();
                self.update_target_bits(target, &vec![BitDeterminacy::Determinate; width]);
            }
            argument_values.push(argument.value);
        }
        Ok(CheckedValue {
            type_: result.clone(),
            bits: vec![BitDeterminacy::Determinate; result.width()],
            extern_call: None,
            reference_target: None,
            value: Value::Call(label.clone(), argument_values),
        })
    }

    fn lookup_local(&self, label: &Label) -> Result<&LocalBinding, CheckError> {
        self.scope
            .get(label)
            .ok_or_else(|| CheckError::at(CheckErrorKind::UnknownLabel, label))
    }

    fn target_bits(&self, target: ReferenceTarget, width: usize) -> Vec<BitDeterminacy> {
        let bits = &self.slot_bits[target.slot.index()];
        if target.bit_offset + width <= bits.len() {
            bits[target.bit_offset..target.bit_offset + width].to_vec()
        } else {
            vec![BitDeterminacy::Determinate; width]
        }
    }

    fn update_target_bits(&mut self, target: ReferenceTarget, bits: &[BitDeterminacy]) {
        let slot_bits = &mut self.slot_bits[target.slot.index()];
        if target.bit_offset + bits.len() <= slot_bits.len() {
            slot_bits[target.bit_offset..target.bit_offset + bits.len()].clone_from_slice(bits);
        }
    }

    fn as_type_expression(&self, expression: &Expression) -> Option<TypeExpression> {
        match expression {
            Expression::Bit => Some(TypeExpression::Bit),
            Expression::Label(label) => match self.globals.get(label) {
                Some(GlobalBinding::Type(_)) => Some(TypeExpression::Label(label.clone())),
                _ => None,
            },
            Expression::Reference(label) => match self.globals.get(label) {
                Some(GlobalBinding::Type(_)) => Some(TypeExpression::Reference(Box::new(
                    TypeExpression::Label(label.clone()),
                ))),
                _ => None,
            },
            Expression::Collection(elements) => {
                let mut type_expressions = Vec::new();
                for element in elements {
                    type_expressions.push(self.as_type_expression(element)?);
                }
                Some(TypeExpression::Collection(type_expressions))
            }
            _ => None,
        }
    }

    fn resolve_type(&self, type_expression: &TypeExpression) -> Result<Type, CheckError> {
        match type_expression {
            TypeExpression::Bit => Ok(Type::Bit),
            TypeExpression::Label(label) => match self.globals.get(label) {
                Some(GlobalBinding::Type(type_)) => Ok(type_.clone()),
                Some(_) => Err(CheckError::at(CheckErrorKind::ExpectedType, label)),
                None => Err(CheckError::at(CheckErrorKind::UnknownLabel, label)),
            },
            TypeExpression::Reference(inner) => {
                Ok(Type::Reference(Box::new(self.resolve_type(inner)?)))
            }
            TypeExpression::Collection(elements) => {
                let mut types = Vec::new();
                for element in elements {
                    types.push(self.resolve_type(element)?);
                }
                Ok(Type::Collection(types))
            }
        }
    }
}

struct DestructuringSource {
    value: Value,
    slot: Option<Slot>,
    bits: Vec<BitDeterminacy>,
    type_: Type,
    bit_offset: usize,
}

fn source_needs_slot(targets: &[Target], value: &Value) -> bool {
    target_count(targets) > 1
        || targets.iter().any(target_takes_reference)
        || value_loses_its_type(value)
}

fn value_loses_its_type(value: &Value) -> bool {
    match value {
        Value::Constant(_) | Value::Branch(..) => true,
        Value::Collection(elements) => elements.iter().any(value_loses_its_type),
        Value::Element { collection, .. } => value_loses_its_type(collection),
        _ => false,
    }
}

fn target_count(targets: &[Target]) -> usize {
    targets
        .iter()
        .map(|target| match target {
            Target::Destructuring(nested) => target_count(nested),
            Target::Named(_) | Target::Reference(_) => 1,
        })
        .sum()
}

fn target_takes_reference(target: &Target) -> bool {
    match target {
        Target::Reference(_) => true,
        Target::Destructuring(targets) => targets.iter().any(target_takes_reference),
        Target::Named(_) => false,
    }
}

fn element_bit_offset(type_: &Type, index: usize) -> Result<usize, CheckError> {
    let elements = type_
        .elements()
        .ok_or_else(|| CheckError::new(CheckErrorKind::NotDestructurable))?;
    if index >= elements.len() {
        return Err(CheckError::new(CheckErrorKind::WrongElementCount));
    }
    Ok(elements[..index].iter().map(Type::width).sum())
}

fn discharge_guard(bits: &[BitDeterminacy], guard: Option<usize>) -> Vec<BitDeterminacy> {
    let Some(guard) = guard else {
        return bits.to_vec();
    };
    bits.iter()
        .map(|bit| match bit {
            BitDeterminacy::Guarded(calls) => {
                let remaining: BTreeSet<usize> = calls
                    .iter()
                    .copied()
                    .filter(|call| *call != guard)
                    .collect();
                if remaining.is_empty() {
                    BitDeterminacy::Determinate
                } else {
                    BitDeterminacy::Guarded(remaining)
                }
            }
            other => other.clone(),
        })
        .collect()
}

fn empty_value() -> CheckedValue {
    CheckedValue {
        type_: Type::Collection(Vec::new()),
        bits: Vec::new(),
        extern_call: None,
        reference_target: None,
        value: Value::Collection(Vec::new()),
    }
}

fn merge_determinacy(left: &[BitDeterminacy], right: &[BitDeterminacy]) -> Vec<BitDeterminacy> {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| {
            if left == right {
                return left.clone();
            }
            match (left.guard_requirements(), right.guard_requirements()) {
                (Some(left), Some(right)) => {
                    let union: BTreeSet<usize> = left.union(&right).copied().collect();
                    if union.is_empty() {
                        BitDeterminacy::Determinate
                    } else {
                        BitDeterminacy::Guarded(union)
                    }
                }
                _ => BitDeterminacy::Unknown,
            }
        })
        .collect()
}

fn bits_to_u16(bits: &[bool]) -> u16 {
    let mut value = 0u16;
    for (index, bit) in bits.iter().enumerate() {
        if *bit {
            value |= 1 << index;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use lapc_parse::parse_program;

    fn check(source: &str) -> Result<IrProgram, CheckError> {
        let program = parse_program(&format!("{PRELUDE}{source}")).expect("the source parses");
        check_program(&program)
    }

    const PRELUDE: &str =
        "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\n";

    #[test]
    fn constants_and_entry_check() {
        let program = check("main = () BIT { BIT.ZERO }\n").expect("the program checks");
        assert_eq!(program.functions().len(), 1);
        assert_eq!(program.functions()[0].label().text(), "main");
    }

    #[test]
    fn a_function_with_a_parameter_checks() {
        let program = check(
            "bit.not = (a: BIT) BIT { NAND(a, a) }\n\
             main = () BIT { bit.not(BIT.ONE) }\n",
        )
        .expect("the program checks");
        assert_eq!(program.functions().len(), 2);
    }

    #[test]
    fn an_erased_function_is_declared_with_an_empty_collection() {
        let program = check(
            "liblapc.bit.not = []\n\
             main = () BIT { liblapc.bit.not(BIT.ONE) }\n",
        )
        .expect("the program checks");
        let erased = function(&program, "liblapc.bit.not");
        assert_eq!(erased.parameters().len(), 1);
        assert_eq!(erased.result(), &Type::Bit);
    }

    #[test]
    fn an_erased_function_derives_its_signature_from_its_label() {
        let source = format!(
            "U64 = [{}]\nliblapc.u64.div.mod = []\nmain = () BIT {{ BIT.ZERO }}\n",
            type_list(64)
        );
        let program = check(&source).expect("the program checks");
        let erased = function(&program, "liblapc.u64.div.mod");
        assert_eq!(erased.parameters().len(), 2);
        assert_eq!(
            erased.result(),
            &Type::Collection(vec![
                Type::Collection(vec![Type::Bit; 64]),
                Type::Collection(vec![Type::Bit; 64]),
            ])
        );
    }

    #[test]
    fn an_empty_collection_outside_the_table_still_binds_a_type() {
        let program = check(
            "EMPTY = []\n\
             nothing = () EMPTY { [] }\n\
             main = () BIT { nothing()\n BIT.ZERO }\n",
        )
        .expect("the program checks");
        assert_eq!(program.functions().len(), 2);
    }

    #[test]
    fn a_reference_parameter_checks() {
        let program = check(
            "bit.not = (a: BIT) BIT { NAND(a, a) }\n\
             bit.toggle = (target: *BIT) [] { target = bit.not(target) }\n\
             main = () BIT { state: BIT = BIT.ZERO\n bit.toggle(*state)\n state }\n",
        )
        .expect("the program checks");
        assert_eq!(program.functions().len(), 3);
    }

    #[test]
    fn a_destructuring_with_a_reference_target_checks() {
        let program = check(
            "U2 = [BIT, BIT]\n\
             TUPLE = [BIT, U2]\n\
             main = () BIT { bundle: TUPLE = [BIT.ZERO, [BIT.ZERO, BIT.ONE]]\n [head, *middle] = bundle\n head }\n",
        )
        .expect("the program checks");
        assert_eq!(program.functions().len(), 1);
    }

    #[test]
    fn a_destructuring_of_an_extern_binds_the_source_once() {
        let program = check_with_extern(
            "main = () BIT { [status, byte] = EXTERN(OP.RANDOM.BYTE)\n BIT.ZERO }\n",
        )
        .expect("the program checks");
        assert_eq!(extern_count(function(&program, "main")), 1);
    }

    #[test]
    fn a_nested_destructuring_of_an_extern_binds_the_source_once() {
        let program = check_with_extern(
            "main = () BIT { [[status, byte]] = [EXTERN(OP.RANDOM.BYTE)]\n BIT.ZERO }\n",
        )
        .expect("the program checks");
        assert_eq!(extern_count(function(&program, "main")), 1);
    }

    #[test]
    fn a_single_target_destructuring_keeps_the_source_inline() {
        let program = check_with_extern(
            "main = () BIT { [status] = EXTERN(OP.STREAM.FLUSH, U64.ZERO)\n BIT.ZERO }\n",
        )
        .expect("the program checks");
        let statements = function(&program, "main").body().statements();
        assert_eq!(statements.len(), 1);
        assert!(matches!(
            &statements[0],
            IrStatement::Bind {
                value: Value::Element { .. },
                ..
            }
        ));
    }

    #[test]
    fn a_single_target_destructuring_of_a_constant_binds_the_source() {
        let program = check(
            "ONE = [BIT]\n\
             ONE.ONE = [BIT.ONE]\n\
             main = () BIT { [x] = ONE.ONE\n x }\n",
        )
        .expect("the program checks");
        let statements = function(&program, "main").body().statements();
        assert_eq!(statements.len(), 2);
        assert!(matches!(
            &statements[0],
            IrStatement::Bind {
                value: Value::Constant(_),
                ..
            }
        ));
        assert!(matches!(
            &statements[1],
            IrStatement::Bind {
                value: Value::Element { .. },
                ..
            }
        ));
    }

    #[test]
    fn an_extern_call_checks_and_guards_its_payload() {
        let program = check(
            "OP.RANDOM.BYTE = [BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO]\n\
             U8 = [BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]\n\
             u8.identity = (value: U8) U8 { value }\n\
             main = () BIT { [status, byte] = EXTERN(OP.RANDOM.BYTE)\n BRANCH (status) { checked: U8 = u8.identity(byte)\n BIT.ONE } { BIT.ZERO } }\n",
        )
        .expect("the program checks");
        assert_eq!(program.functions().len(), 2);
    }

    #[test]
    fn an_indeterminate_condition_is_rejected() {
        let error = check("main = () BIT { BRANCH (BIT) { BIT.ZERO } { BIT.ZERO } }\n")
            .expect_err("the condition is indeterminate");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn a_branch_on_a_status_bit_discharges_the_guard() {
        let program = check_with_extern(
            "main = () BIT { [status, byte] = EXTERN(OP.RANDOM.BYTE)\n value: U8 = BRANCH (status) { byte } { U8.ZERO }\n checked: U8 = u8.identity(value)\n BIT.ZERO }\n",
        )
        .expect("the program checks");
        assert_eq!(program.functions().len(), 2);
    }

    #[test]
    fn a_bound_extern_result_keeps_its_guard() {
        let program = check_with_extern(
            "main = () BIT { result: [BIT, U8] = EXTERN(OP.RANDOM.BYTE)\n [status, byte] = result\n BRANCH (status) { checked: U8 = u8.identity(byte)\n BIT.ONE } { BIT.ZERO } }\n",
        )
        .expect("the program checks");
        assert_eq!(program.functions().len(), 2);
    }

    #[test]
    fn a_branch_on_a_constant_zero_uses_the_else_block() {
        check("main = () BIT { BRANCH (BIT.ZERO) { BIT } { BIT.ZERO } }\n")
            .expect("the dead block is not the result");
    }

    #[test]
    fn a_branch_on_a_constant_one_uses_the_then_block() {
        check("main = () BIT { BRANCH (BIT.ONE) { BIT.ZERO } { BIT } }\n")
            .expect("the dead block is not the result");
    }

    #[test]
    fn a_branch_block_ending_in_a_binding_yields_an_empty_collection() {
        let program = check(
            "main = () BIT { state: BIT = BIT.ZERO\n BRANCH (state) { first: BIT = BIT.ONE } { second: BIT = BIT.ZERO }\n BIT.ZERO }\n",
        )
        .expect("the program checks");
        let statements = function(&program, "main").body().statements();
        let (then, otherwise) = match &statements[1] {
            IrStatement::Evaluate {
                value: Value::Branch(_, then, otherwise),
            } => (then, otherwise),
            other => panic!("the branch is not evaluated: {other:?}"),
        };
        assert_eq!(then.result(), Some(&Value::Collection(Vec::new())));
        assert_eq!(otherwise.result(), Some(&Value::Collection(Vec::new())));
    }

    #[test]
    fn a_branch_with_an_empty_block_and_a_bit_block_is_rejected() {
        let error = check(
            "main = () BIT { state: BIT = BIT.ZERO\n BRANCH (state) { first: BIT = BIT.ONE } { BIT.ZERO }\n BIT.ZERO }\n",
        )
        .expect_err("the blocks have different types");
        assert_eq!(error.kind(), &CheckErrorKind::TypeMismatch);
    }

    #[test]
    fn an_unchecked_payload_is_rejected() {
        let error = check(
            "OP.RANDOM.BYTE = [BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO]\n\
             U8 = [BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]\n\
             u8.identity = (value: U8) U8 { value }\n\
             main = () BIT { [status, byte] = EXTERN(OP.RANDOM.BYTE)\n checked: U8 = u8.identity(byte)\n BIT.ZERO }\n",
        )
        .expect_err("the payload is unchecked");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn a_shadowed_global_is_rejected() {
        let error = check("main = () BIT { BIT.ONE: BIT = BIT.ZERO\n BIT.ONE }\n")
            .expect_err("the global is shadowed");
        assert_eq!(error.kind(), &CheckErrorKind::ShadowedGlobal);
    }

    #[test]
    fn a_nand_of_constants_keeps_the_nand() {
        let program =
            check("main = () BIT { NAND(BIT.ZERO, BIT.ZERO) }\n").expect("the program checks");
        assert_eq!(
            program.functions()[0].body().result(),
            Some(&Value::Nand(
                Box::new(Value::Constant(BitVector::new(vec![false]))),
                Box::new(Value::Constant(BitVector::new(vec![false]))),
            )),
        );
    }

    fn type_list(width: usize) -> String {
        vec!["BIT"; width].join(", ")
    }

    fn bit_list(width: usize, set_bits: &[usize]) -> String {
        (0..width)
            .map(|index| {
                if set_bits.contains(&index) {
                    "BIT.ONE"
                } else {
                    "BIT.ZERO"
                }
            })
            .collect::<Vec<&str>>()
            .join(", ")
    }

    fn operation_constant(name: &str, code: u16) -> String {
        let set_bits: Vec<usize> = (0..16).filter(|index| code & (1 << index) != 0).collect();
        format!("{name} = [{}]\n", bit_list(16, &set_bits))
    }

    fn extern_prelude() -> String {
        let mut source = String::new();
        source.push_str(&operation_constant("OP.RANDOM.BYTE", 0x0400));
        source.push_str(&operation_constant("OP.STREAM.FLUSH", 0x0203));
        source.push_str(&operation_constant("OP.UNKNOWN", 0x7fff));
        source.push_str(&format!("U8 = [{}]\n", type_list(8)));
        source.push_str(&format!("U8.ZERO = [{}]\n", bit_list(8, &[])));
        source.push_str(&format!("U64 = [{}]\n", type_list(64)));
        source.push_str(&format!("U64.ZERO = [{}]\n", bit_list(64, &[])));
        source.push_str("u8.identity = (value: U8) U8 { value }\n");
        source
    }

    fn check_with_extern(source: &str) -> Result<IrProgram, CheckError> {
        check(&format!("{}{source}", extern_prelude()))
    }

    fn function<'program>(program: &'program IrProgram, label: &str) -> &'program Function {
        program
            .functions()
            .iter()
            .find(|function| function.label().text() == label)
            .expect("the function exists")
    }

    fn extern_count(function: &Function) -> usize {
        extern_count_block(function.body())
    }

    fn extern_count_block(block: &IrBlock) -> usize {
        block
            .statements()
            .iter()
            .map(|statement| match statement {
                IrStatement::Bind { value, .. } | IrStatement::Evaluate { value } => {
                    extern_count_value(value)
                }
                IrStatement::Store { reference, value } => {
                    extern_count_value(reference) + extern_count_value(value)
                }
            })
            .sum::<usize>()
            + block.result().map_or(0, extern_count_value)
    }

    fn extern_count_value(value: &Value) -> usize {
        match value {
            Value::Extern(..) => 1,
            Value::Nand(left, right) => extern_count_value(left) + extern_count_value(right),
            Value::Intrinsic(_, arguments) => arguments.iter().map(extern_count_value).sum(),
            Value::Collection(elements) => elements.iter().map(extern_count_value).sum(),
            Value::Element { collection, .. } => extern_count_value(collection),
            Value::Load(reference) => extern_count_value(reference),
            Value::Call(_, arguments) => arguments.iter().map(extern_count_value).sum(),
            Value::Branch(condition, then, otherwise) => {
                extern_count_value(condition)
                    + extern_count_block(then)
                    + extern_count_block(otherwise)
            }
            Value::Constant(_) | Value::Reference { .. } => 0,
        }
    }

    #[test]
    fn a_bare_bit_is_indeterminate() {
        let error =
            check("main = () BIT { NAND(BIT, BIT) }\n").expect_err("the result is indeterminate");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn a_nand_with_zero_is_determinate() {
        check("main = () BIT { NAND(BIT.ZERO, BIT) }\n").expect("the program checks");
    }

    #[test]
    fn a_nand_with_one_and_bit_is_indeterminate() {
        let error = check("main = () BIT { NAND(BIT.ONE, BIT) }\n")
            .expect_err("the result is indeterminate");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn a_payload_in_the_failure_branch_is_rejected() {
        let error = check_with_extern(
            "main = () BIT { [status, byte] = EXTERN(OP.RANDOM.BYTE)\n BRANCH (status) { BIT.ONE } { checked: U8 = u8.identity(byte)\n BIT.ZERO } }\n",
        )
        .expect_err("the payload is unchecked in the failure branch");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn a_payload_after_the_branch_is_rejected() {
        let error = check_with_extern(
            "main = () BIT { [status, byte] = EXTERN(OP.RANDOM.BYTE)\n BRANCH (status) { BIT.ONE } { BIT.ZERO }\n checked: U8 = u8.identity(byte)\n BIT.ZERO }\n",
        )
        .expect_err("the payload is unchecked after the branch");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn a_payload_escaping_the_guard_is_rejected() {
        let error = check_with_extern(
            "main = () BIT { [status, byte] = EXTERN(OP.RANDOM.BYTE)\n value: U8 = BRANCH (status) { byte } { byte }\n checked: U8 = u8.identity(value)\n BIT.ZERO }\n",
        )
        .expect_err("the payload escapes the guard");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn a_nested_guard_checks() {
        let program = check_with_extern(
            "main = () BIT { [outer, first] = EXTERN(OP.RANDOM.BYTE)\n BRANCH (outer) { [inner, second] = EXTERN(OP.RANDOM.BYTE)\n BRANCH (inner) { checked: U8 = u8.identity(second)\n BIT.ONE } { BIT.ZERO } } { BIT.ZERO } }\n",
        )
        .expect("the program checks");
        assert_eq!(program.functions().len(), 2);
    }

    #[test]
    fn a_reference_to_an_indeterminate_local_is_rejected() {
        let error = check(
            "bit.identity = (target: *BIT) BIT { target }\nmain = () BIT { state: BIT = BIT\n bit.identity(*state) }\n",
        )
        .expect_err("the pointee is indeterminate");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn a_reference_to_a_determinate_local_checks() {
        check(
            "bit.identity = (target: *BIT) BIT { target }\nmain = () BIT { state: BIT = BIT.ZERO\n bit.identity(*state) }\n",
        )
        .expect("the program checks");
    }

    #[test]
    fn a_store_of_an_indeterminate_value_is_rejected() {
        let error = check(
            "bit.store = (target: *BIT) [] { target = BIT }\nmain = () BIT { state: BIT = BIT.ZERO\n bit.store(*state)\n state }\n",
        )
        .expect_err("the store is indeterminate");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn a_reference_result_is_rejected() {
        let error =
            check("bit.identity = (target: *BIT) *BIT { target }\nmain = () BIT { BIT.ZERO }\n")
                .expect_err("a reference cannot be returned");
        assert_eq!(error.kind(), &CheckErrorKind::ReferenceResult);
    }

    #[test]
    fn a_reference_in_a_collection_is_rejected() {
        let error = check(
            "main = () BIT { state: BIT = BIT.ZERO\n bundle: [*BIT] = [*state]\n BIT.ZERO }\n",
        )
        .expect_err("a reference cannot be stored in a collection");
        assert_eq!(error.kind(), &CheckErrorKind::ReferenceInCollection);
    }

    #[test]
    fn a_reference_argument_to_extern_is_rejected() {
        let error = check_with_extern(
            "main = () BIT { state: U64 = U64.ZERO\n [status] = EXTERN(OP.STREAM.FLUSH, *state)\n BIT.ZERO }\n",
        )
        .expect_err("a reference cannot be an extern argument");
        assert_eq!(error.kind(), &CheckErrorKind::ReferenceArgument);
    }

    #[test]
    fn an_unknown_operation_is_rejected() {
        let error =
            check_with_extern("main = () BIT { [status] = EXTERN(OP.UNKNOWN)\n BIT.ZERO }\n")
                .expect_err("the operation is unknown");
        assert_eq!(error.kind(), &CheckErrorKind::UnknownOperation);
    }

    #[test]
    fn a_wrong_argument_count_is_rejected() {
        let error = check_with_extern(
            "main = () BIT { [status] = EXTERN(OP.RANDOM.BYTE, BIT.ZERO)\n BIT.ZERO }\n",
        )
        .expect_err("the argument count is wrong");
        assert_eq!(error.kind(), &CheckErrorKind::WrongArgumentCount);
    }

    #[test]
    fn a_width_mismatch_is_rejected() {
        let error = check_with_extern(
            "main = () BIT { [status] = EXTERN(OP.STREAM.FLUSH, BIT.ZERO)\n BIT.ZERO }\n",
        )
        .expect_err("the argument width is wrong");
        assert_eq!(error.kind(), &CheckErrorKind::WidthMismatch);
    }

    #[test]
    fn a_non_constant_operation_is_rejected() {
        let error = check_with_extern("main = () BIT { [status] = EXTERN(BIT)\n BIT.ZERO }\n")
            .expect_err("the operation is not a constant");
        assert_eq!(error.kind(), &CheckErrorKind::NotAConstant);
    }

    #[test]
    fn a_type_mismatch_is_rejected() {
        let error = check("U2 = [BIT, BIT]\nmain = () BIT { value: U2 = BIT.ZERO\n BIT.ZERO }\n")
            .expect_err("the types differ");
        assert_eq!(error.kind(), &CheckErrorKind::TypeMismatch);
    }

    #[test]
    fn a_structural_match_checks() {
        check(
            "U2 = [BIT, BIT]\nPAIR = [BIT, BIT]\nmain = () BIT { value: U2 = [BIT.ZERO, BIT.ONE]\n paired: PAIR = value\n BIT.ZERO }\n",
        )
        .expect("the program checks");
    }

    #[test]
    fn a_global_shadowed_by_a_parameter_is_rejected() {
        let error =
            check("bit.identity = (BIT.ONE: BIT) BIT { BIT.ONE }\nmain = () BIT { BIT.ZERO }\n")
                .expect_err("the global is shadowed by a parameter");
        assert_eq!(error.kind(), &CheckErrorKind::ShadowedGlobal);
    }

    #[test]
    fn a_global_redefined_at_the_top_level_is_rejected() {
        let error = check("BIT.ONE = BIT.ZERO\nmain = () BIT { BIT.ZERO }\n")
            .expect_err("the global is redefined");
        assert_eq!(error.kind(), &CheckErrorKind::ShadowedGlobal);
    }

    #[test]
    fn a_use_before_the_binding_is_rejected() {
        let error = check("main = () BIT { late }\nlate = BIT.ONE\n")
            .expect_err("the name is not visible yet");
        assert_eq!(error.kind(), &CheckErrorKind::UnknownLabel);
    }

    #[test]
    fn a_mutual_recursion_is_rejected() {
        let error = check(
            "first = () BIT { second() }\nsecond = () BIT { first() }\nmain = () BIT { BIT.ZERO }\n",
        )
        .expect_err("the second function is not visible yet");
        assert_eq!(error.kind(), &CheckErrorKind::UnknownLabel);
    }

    #[test]
    fn a_self_recursive_function_checks() {
        check(
            "count.down = (n: BIT) BIT { BRANCH (n) { count.down(BIT.ZERO) } { BIT.ZERO } }\nmain = () BIT { count.down(BIT.ONE) }\n",
        )
        .expect("the program checks");
    }

    #[test]
    fn an_entry_with_parameters_is_rejected() {
        let error = check("main = (a: BIT) BIT { a }\n").expect_err("the entry takes parameters");
        assert_eq!(error.kind(), &CheckErrorKind::EntrySignature);
    }

    #[test]
    fn an_entry_returning_a_non_bit_is_rejected() {
        let error = check("U2 = [BIT, BIT]\nmain = () U2 { [BIT.ZERO, BIT.ONE] }\n")
            .expect_err("the entry does not return a bit");
        assert_eq!(error.kind(), &CheckErrorKind::EntrySignature);
    }

    #[test]
    fn a_top_level_call_is_rejected() {
        let error = check(
            "bit.not = (a: BIT) BIT { NAND(a, a) }\nvalue = bit.not(BIT.ZERO)\nmain = () BIT { BIT.ZERO }\n",
        )
        .expect_err("a top-level value is not a constant");
        assert_eq!(error.kind(), &CheckErrorKind::NotAConstant);
    }

    #[test]
    fn a_top_level_expression_is_rejected() {
        let error = check("bit.not(BIT.ZERO)\nmain = () BIT { BIT.ZERO }\n")
            .expect_err("a top-level expression is not a binding");
        assert_eq!(error.kind(), &CheckErrorKind::TopLevelExpression);
    }

    #[test]
    fn a_wrong_element_count_is_rejected() {
        let error = check("main = () BIT { [first] = [BIT.ZERO, BIT.ONE]\n BIT.ZERO }\n")
            .expect_err("the element count is wrong");
        assert_eq!(error.kind(), &CheckErrorKind::WrongElementCount);
    }

    #[test]
    fn a_non_collection_destructuring_is_rejected() {
        let error = check("main = () BIT { [first] = BIT.ZERO\n BIT.ZERO }\n")
            .expect_err("the value is not a collection");
        assert_eq!(error.kind(), &CheckErrorKind::NotDestructurable);
    }

    #[test]
    fn a_nested_destructuring_checks() {
        check(
            "NESTED = [[BIT, BIT], BIT]\nmain = () BIT { bundle: NESTED = [[BIT.ZERO, BIT.ONE], BIT.ONE]\n [[first, second], third] = bundle\n first }\n",
        )
        .expect("the program checks");
    }

    #[test]
    fn a_nested_reference_target_checks() {
        check(
            "NESTED = [[BIT, BIT], BIT]\nmain = () BIT { bundle: NESTED = [[BIT.ZERO, BIT.ONE], BIT.ONE]\n [[first, *second], third] = bundle\n first }\n",
        )
        .expect("the program checks");
    }

    #[test]
    fn a_destructuring_annotation_mismatch_is_rejected() {
        let error = check(
            "U2 = [BIT, BIT]\nmain = () BIT { [first: U2, second: BIT] = [BIT.ZERO, BIT.ONE]\n BIT.ZERO }\n",
        )
        .expect_err("the annotation does not match");
        assert_eq!(error.kind(), &CheckErrorKind::TypeMismatch);
    }

    #[test]
    fn a_block_ending_in_a_binding_with_a_value_result_is_rejected() {
        let error = check(
            "bit.set = (target: *BIT) BIT { target = BIT.ZERO }\nmain = () BIT { BIT.ZERO }\n",
        )
        .expect_err("the block has no result");
        assert_eq!(error.kind(), &CheckErrorKind::TypeMismatch);
    }

    #[test]
    fn an_indeterminate_function_result_is_rejected() {
        let error = check("bit.unknown = () BIT { BIT }\nmain = () BIT { BIT.ZERO }\n")
            .expect_err("the result is indeterminate");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn a_call_with_a_reference_argument_invalidates_the_target() {
        let error = check(
            "bit.set = (target: *BIT) [] { target = BIT.ONE }\nmain = () BIT { state: BIT = BIT.ZERO\n bit.set(*state)\n BRANCH (state) { BIT.ZERO } { BIT } }\n",
        )
        .expect_err("the callee may have changed the target");
        assert_eq!(error.kind(), &CheckErrorKind::Indeterminate);
    }

    #[test]
    fn an_empty_result_checks() {
        check(
            "bit.toggle = (target: *BIT) [] { target = BIT.ZERO }\nmain = () BIT { state: BIT = BIT.ZERO\n bit.toggle(*state)\n state }\n",
        )
        .expect("the program checks");
    }

    #[test]
    fn a_binding_produces_a_slot_and_a_load() {
        let program =
            check("main = () BIT { state: BIT = BIT.ONE\n state }\n").expect("the program checks");
        let function = &program.functions()[0];
        assert_eq!(function.slots(), &[Type::Bit]);
        assert_eq!(
            function.body().statements(),
            &[IrStatement::Bind {
                slot: Slot::new(0),
                value: Value::Constant(BitVector::new(vec![true])),
            }],
        );
        assert_eq!(
            function.body().result(),
            Some(&Value::Load(Box::new(Value::Reference {
                slot: Slot::new(0),
                bit_offset: 0
            }))),
        );
    }

    #[test]
    fn a_missing_entry_is_rejected() {
        let error = check("bit.not = (a: BIT) BIT { NAND(a, a) }\n").expect_err("main is missing");
        assert_eq!(error.kind(), &CheckErrorKind::MissingEntry);
    }
}
