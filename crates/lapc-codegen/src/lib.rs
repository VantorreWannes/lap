use std::collections::HashMap;

use cranelift_codegen::Context;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    AbiParam, Block as CraneliftBlock, BlockArg, Endianness, InstBuilder, MemFlagsData, Signature,
    StackSlot, StackSlotData, StackSlotKind, TrapCode, Type as CraneliftType,
    Value as CraneliftValue, types,
};
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};
use lapc_extern::{Operation, OperationSpecification, lookup as lookup_operation};
use lapc_ir::{BitVector, Block, Function, Intrinsic, Program, Slot, Statement, Type, Value};

const STACK_SLOT_SLACK: usize = 8;
const STACK_SLOT_ALIGNMENT_SHIFT: u8 = 3;
const EXTERN_OUTPUT_SIZE: usize = 2 * STACK_SLOT_SLACK;

pub fn emit_object(program: &Program) -> Result<Vec<u8>, CodegenError> {
    let instruction_set = host_instruction_set()?;
    let builder = ObjectBuilder::new(instruction_set, "lap", default_libcall_names())
        .map_err(|error| CodegenError::new(error.to_string()))?;
    let mut module = ObjectModule::new(builder);
    lower_program(&mut module, program)?;
    module
        .finish()
        .emit()
        .map_err(|error| CodegenError::new(error.to_string()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodegenError {
    message: String,
}

impl CodegenError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CodegenError {}

fn host_instruction_set() -> Result<OwnedTargetIsa, CodegenError> {
    let flags = settings::Flags::new(settings::builder());
    cranelift_codegen::isa::lookup(target_lexicon::HOST)
        .map_err(|error| CodegenError::new(error.to_string()))?
        .finish(flags)
        .map_err(|error| CodegenError::new(error.to_string()))
}

fn lower_program<M: Module>(
    module: &mut M,
    program: &Program,
) -> Result<HashMap<String, FuncId>, CodegenError> {
    let mut functions = HashMap::new();
    for function in program.functions() {
        let signature = function_signature(module, function);
        let identifier = module
            .declare_function(
                &export_name(function.label().text()),
                function_linkage(function.label().text()),
                &signature,
            )
            .map_err(|error| CodegenError::new(error.to_string()))?;
        functions.insert(
            function.label().text(),
            FunctionSymbol {
                function,
                identifier,
            },
        );
    }
    let extern_signature = extern_signature(module);
    let extern_identifier = module
        .declare_function("lap_extern", Linkage::Import, &extern_signature)
        .map_err(|error| CodegenError::new(error.to_string()))?;
    let symbols = ProgramSymbols {
        functions,
        extern_identifier,
    };
    let mut context = module.make_context();
    let mut builder_context = FunctionBuilderContext::new();
    for function in program.functions() {
        let identifier = symbols
            .functions
            .get(function.label().text())
            .map(|symbol| symbol.identifier)
            .ok_or_else(|| CodegenError::new("a function is not declared"))?;
        lower_function(
            module,
            &mut context,
            &mut builder_context,
            function,
            &symbols,
        )?;
        module
            .define_function(identifier, &mut context)
            .map_err(|error| CodegenError::new(error.to_string()))?;
        module.clear_context(&mut context);
    }
    Ok(symbols
        .functions
        .iter()
        .map(|(label, symbol)| ((*label).to_string(), symbol.identifier))
        .collect())
}

fn function_signature<M: Module>(module: &M, function: &Function) -> Signature {
    let mut signature = module.make_signature();
    if function.result().width() > 64 {
        signature.params.push(AbiParam::new(types::I64));
    }
    for _ in function.parameters() {
        signature.params.push(AbiParam::new(types::I64));
    }
    if function.result().width() <= 64 {
        signature.returns.push(AbiParam::new(types::I64));
    }
    signature
}

fn extern_signature<M: Module>(module: &M) -> Signature {
    let mut signature = module.make_signature();
    for _ in 0..6 {
        signature.params.push(AbiParam::new(types::I64));
    }
    signature
}

fn lower_function<M: Module>(
    module: &mut M,
    context: &mut Context,
    builder_context: &mut FunctionBuilderContext,
    function: &Function,
    symbols: &ProgramSymbols,
) -> Result<(), CodegenError> {
    context.func.signature = function_signature(module, function);
    let mut builder = FunctionBuilder::new(&mut context.func, builder_context);
    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    builder.seal_block(entry);
    let parameters = builder.block_params(entry).to_vec();
    {
        let mut lowering = Lowering::new(&mut builder, module, function, symbols, &parameters)?;
        lowering.lower_body()?;
    }
    builder.finalize(module.target_config());
    Ok(())
}

struct ProgramSymbols<'program> {
    functions: HashMap<&'program str, FunctionSymbol<'program>>,
    extern_identifier: FuncId,
}

struct FunctionSymbol<'program> {
    function: &'program Function,
    identifier: FuncId,
}

#[derive(Clone, Copy)]
enum Lowered {
    Word(CraneliftValue),
    Slot(CraneliftValue),
}

#[derive(Clone, Copy)]
enum WideOperation {
    Nand,
    Not,
    And,
    Or,
    Xor,
    Add,
    Sub,
}

struct Lowering<'builder, 'function, 'program, M: Module> {
    builder: &'builder mut FunctionBuilder<'function>,
    module: &'builder mut M,
    function: &'program Function,
    symbols: &'builder ProgramSymbols<'program>,
    slots: Vec<StackSlot>,
    parameter_values: Vec<CraneliftValue>,
    output_pointer: Option<CraneliftValue>,
    pointer_type: CraneliftType,
    body_block: Option<CraneliftBlock>,
}

impl<'builder, 'function, 'program, M: Module> Lowering<'builder, 'function, 'program, M> {
    fn new(
        builder: &'builder mut FunctionBuilder<'function>,
        module: &'builder mut M,
        function: &'program Function,
        symbols: &'builder ProgramSymbols<'program>,
        parameters: &[CraneliftValue],
    ) -> Result<Self, CodegenError> {
        let pointer_type = module.target_config().pointer_type();
        let mut slots = Vec::new();
        for type_ in function.slots() {
            slots.push(create_stack_slot(
                builder,
                byte_size(type_.width()) + STACK_SLOT_SLACK,
            )?);
        }
        let wide_result = function.result().width() > 64;
        let output_pointer = if wide_result {
            parameters.first().copied()
        } else {
            None
        };
        let parameter_values = if wide_result {
            parameters[1..].to_vec()
        } else {
            parameters.to_vec()
        };
        Ok(Self {
            builder,
            module,
            function,
            symbols,
            slots,
            parameter_values,
            output_pointer,
            pointer_type,
            body_block: None,
        })
    }

    fn lower_body(&mut self) -> Result<(), CodegenError> {
        let function = self.function;
        let parameters = function.parameters().to_vec();
        for (index, parameter) in parameters.iter().enumerate() {
            let parameter_type = function
                .slots()
                .get(parameter.slot().index())
                .ok_or_else(|| CodegenError::new("a parameter has no slot"))?
                .clone();
            let incoming = self
                .parameter_values
                .get(index)
                .copied()
                .ok_or_else(|| CodegenError::new("a parameter has no value"))?;
            let storage = self.storage(parameter.slot())?;
            if parameter_type.is_reference() || parameter_type.width() <= 64 {
                let word = if parameter_type.is_reference() {
                    incoming
                } else {
                    self.mask(incoming, parameter_type.width())
                };
                self.store_word(storage, word);
            } else {
                let destination = self.slot_bit_pointer(storage);
                let source = self.builder.ins().ishl_imm_u(incoming, 3);
                self.copy_bits(source, destination, parameter_type.width())?;
            }
        }
        let result_type = function.result().clone();
        let body_block = self.builder.create_block();
        self.builder.ins().jump(body_block, &[]);
        self.builder.switch_to_block(body_block);
        self.body_block = Some(body_block);
        let result = self.lower_block(function.body(), &result_type, true)?;
        self.builder.seal_block(body_block);
        let result = match result {
            Some(result) => result,
            None => return Ok(()),
        };
        if result_type.width() > 64 {
            let output_pointer = self
                .output_pointer
                .ok_or_else(|| CodegenError::new("a wide result needs an out pointer"))?;
            match result {
                Lowered::Slot(address) => {
                    let source = self.builder.ins().ishl_imm_u(address, 3);
                    let destination = self.builder.ins().ishl_imm_u(output_pointer, 3);
                    self.copy_bits(source, destination, result_type.width())?;
                }
                Lowered::Word(_) => return Err(CodegenError::new("a wide result was expected")),
            }
            self.builder.ins().return_(&[]);
        } else {
            match result {
                Lowered::Word(word) => {
                    self.builder.ins().return_(&[word]);
                }
                Lowered::Slot(_) => return Err(CodegenError::new("a word result was expected")),
            }
        }
        Ok(())
    }

    fn lower_block(
        &mut self,
        block: &Block,
        expected: &Type,
        tail: bool,
    ) -> Result<Option<Lowered>, CodegenError> {
        for statement in block.statements() {
            match statement {
                Statement::Bind { slot, value } => {
                    let type_ = self.slot_type(*slot)?.clone();
                    let lowered = self.lower_value(value, &type_)?;
                    let storage = self.storage(*slot)?;
                    self.write_slot(storage, lowered, type_.width())?;
                }
                Statement::Store { reference, value } => {
                    let reference_type = self.type_of(reference)?;
                    let inner = match reference_type {
                        Type::Reference(inner) => *inner,
                        _ => return Err(CodegenError::new("a store needs a reference")),
                    };
                    let pointer = self.lower_word(reference, 64)?;
                    let lowered = self.lower_value(value, &inner)?;
                    self.write_bits(pointer, lowered, inner.width())?;
                }
                Statement::Evaluate { value } => {
                    let type_ = self.type_of(value)?;
                    self.lower_value(value, &type_)?;
                }
            }
        }
        match block.result() {
            Some(value) => self.lower_result(value, expected, tail),
            None => Ok(Some(Lowered::Word(
                self.builder.ins().iconst(types::I64, 0),
            ))),
        }
    }

    fn lower_result(
        &mut self,
        value: &Value,
        expected: &Type,
        tail: bool,
    ) -> Result<Option<Lowered>, CodegenError> {
        if tail {
            match value {
                Value::Call(label, arguments) => {
                    if self.lower_self_tail_call(label.text(), arguments, expected)? {
                        return Ok(None);
                    }
                }
                Value::Branch(condition, then, otherwise) => {
                    return self.lower_branch(condition, then, otherwise, expected, true);
                }
                _ => {}
            }
        }
        Ok(Some(self.lower_value(value, expected)?))
    }

    fn lower_value(&mut self, value: &Value, expected: &Type) -> Result<Lowered, CodegenError> {
        match value {
            Value::Constant(bits) => self.lower_constant(bits, expected),
            Value::Nand(left, right) => self.lower_nand(left, right, expected),
            Value::Intrinsic(intrinsic, arguments) => {
                self.lower_intrinsic(*intrinsic, arguments, expected)
            }
            Value::Collection(elements) => self.lower_collection(elements, expected),
            Value::Element { collection, index } => {
                self.lower_element(collection, *index, expected)
            }
            Value::Reference { slot, bit_offset } => {
                self.lower_reference(*slot, *bit_offset, expected)
            }
            Value::Load(reference) => self.lower_load(reference, expected),
            Value::Call(label, arguments) => self.lower_call(label.text(), arguments, expected),
            Value::Extern(operation, arguments) => {
                self.lower_extern(*operation, arguments, expected)
            }
            Value::Branch(condition, then, otherwise) => {
                match self.lower_branch(condition, then, otherwise, expected, false)? {
                    Some(lowered) => Ok(lowered),
                    None => Err(CodegenError::new("a branch produced no value")),
                }
            }
        }
    }

    fn lower_word(&mut self, value: &Value, width: usize) -> Result<CraneliftValue, CodegenError> {
        let type_ = self.type_of(value)?;
        if type_.width() != width {
            return Err(CodegenError::new("an operand has the wrong width"));
        }
        match self.lower_value(value, &type_)? {
            Lowered::Word(word) => Ok(word),
            Lowered::Slot(_) => Err(CodegenError::new("a word was expected")),
        }
    }

    fn lower_constant(
        &mut self,
        bits: &BitVector,
        expected: &Type,
    ) -> Result<Lowered, CodegenError> {
        let width = bits.width();
        if width != expected.width() {
            return Err(CodegenError::new("a constant has the wrong width"));
        }
        if width <= 64 {
            let word = bits_to_word(bits.bits(), 0, width);
            Ok(Lowered::Word(self.builder.ins().iconst(types::I64, word)))
        } else {
            let slot = self.allocate_value_slot(width)?;
            let destination = self.slot_bit_pointer(slot);
            let mut offset = 0;
            while offset < width {
                let chunk = (width - offset).min(64);
                let word = bits_to_word(bits.bits(), offset, chunk);
                let pointer = self.builder.ins().iadd_imm_u(destination, offset as i64);
                let constant = self.builder.ins().iconst(types::I64, word);
                self.store_bits(pointer, constant, chunk)?;
                offset += chunk;
            }
            Ok(Lowered::Slot(self.builder.ins().stack_addr(
                self.pointer_type,
                slot,
                0,
            )))
        }
    }

    fn lower_nand(
        &mut self,
        left: &Value,
        right: &Value,
        expected: &Type,
    ) -> Result<Lowered, CodegenError> {
        let width = expected.width();
        if width <= 64 {
            let left_word = self.lower_word(left, width)?;
            let right_word = self.lower_word(right, width)?;
            let banded = self.builder.ins().band(left_word, right_word);
            let word = self.builder.ins().bnot(banded);
            Ok(Lowered::Word(self.mask(word, width)))
        } else {
            let left_value = self.lower_wide_argument(left, width)?;
            let right_value = self.lower_wide_argument(right, width)?;
            self.lower_wide_operation(WideOperation::Nand, &[left_value, right_value], width)
        }
    }

    fn lower_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        arguments: &[Value],
        expected: &Type,
    ) -> Result<Lowered, CodegenError> {
        if arguments.len() != intrinsic.arity() {
            return Err(CodegenError::new("an intrinsic has the wrong arity"));
        }
        let result_width = expected.width();
        let operand_width = if intrinsic.is_comparison() {
            self.type_of(&arguments[0])?.width()
        } else {
            result_width
        };
        if intrinsic == Intrinsic::Select && operand_width != 1 {
            return Err(CodegenError::new("a select needs a bit condition"));
        }
        if operand_width <= 64 {
            let mut words = Vec::new();
            for argument in arguments {
                words.push(self.lower_word(argument, operand_width)?);
            }
            let word = match intrinsic {
                Intrinsic::Not => self.builder.ins().bnot(words[0]),
                Intrinsic::And => self.builder.ins().band(words[0], words[1]),
                Intrinsic::Or => self.builder.ins().bor(words[0], words[1]),
                Intrinsic::Xor => self.builder.ins().bxor(words[0], words[1]),
                Intrinsic::Add => self.builder.ins().iadd(words[0], words[1]),
                Intrinsic::Sub => self.builder.ins().isub(words[0], words[1]),
                Intrinsic::Mul => self.builder.ins().imul(words[0], words[1]),
                Intrinsic::Inc => self.builder.ins().iadd_imm_u(words[0], 1),
                Intrinsic::Dec => {
                    let one = self.builder.ins().iconst(types::I64, 1);
                    self.builder.ins().isub(words[0], one)
                }
                Intrinsic::ShiftLeftOne => self.builder.ins().ishl_imm_u(words[0], 1),
                Intrinsic::ShiftRightOne => self.builder.ins().ushr_imm_u(words[0], 1),
                Intrinsic::Eq => {
                    let flag = self.builder.ins().icmp(IntCC::Equal, words[0], words[1]);
                    self.builder.ins().uextend(types::I64, flag)
                }
                Intrinsic::Lt => {
                    let flag = self
                        .builder
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, words[0], words[1]);
                    self.builder.ins().uextend(types::I64, flag)
                }
                Intrinsic::IsZero => {
                    let zero = self.builder.ins().iconst(types::I64, 0);
                    let flag = self.builder.ins().icmp(IntCC::Equal, words[0], zero);
                    self.builder.ins().uextend(types::I64, flag)
                }
                Intrinsic::Select => self.builder.ins().select(words[0], words[1], words[2]),
            };
            Ok(Lowered::Word(self.mask(word, result_width)))
        } else {
            let mut values = Vec::new();
            for argument in arguments {
                values.push(self.lower_wide_argument(argument, operand_width)?);
            }
            let operation = match intrinsic {
                Intrinsic::Not => WideOperation::Not,
                Intrinsic::And => WideOperation::And,
                Intrinsic::Or => WideOperation::Or,
                Intrinsic::Xor => WideOperation::Xor,
                Intrinsic::Add => WideOperation::Add,
                Intrinsic::Sub => WideOperation::Sub,
                _ => {
                    return Err(CodegenError::new(
                        "an intrinsic is only defined for widths up to 64",
                    ));
                }
            };
            self.lower_wide_operation(operation, &values, operand_width)
        }
    }

    fn lower_wide_argument(
        &mut self,
        value: &Value,
        width: usize,
    ) -> Result<Lowered, CodegenError> {
        let type_ = self.type_of(value)?;
        if type_.width() != width {
            return Err(CodegenError::new("an operand has the wrong width"));
        }
        self.lower_value(value, &type_)
    }

    fn lower_wide_operation(
        &mut self,
        operation: WideOperation,
        arguments: &[Lowered],
        width: usize,
    ) -> Result<Lowered, CodegenError> {
        let slot = self.allocate_value_slot(width)?;
        let destination = self.slot_bit_pointer(slot);
        let mut carry = self.builder.ins().iconst(types::I64, 0);
        let mut offset = 0;
        while offset < width {
            let chunk = (width - offset).min(64);
            let word = match operation {
                WideOperation::Nand => {
                    let left = self.load_chunk(arguments[0], offset, chunk)?;
                    let right = self.load_chunk(arguments[1], offset, chunk)?;
                    let banded = self.builder.ins().band(left, right);
                    self.builder.ins().bnot(banded)
                }
                WideOperation::Not => {
                    let value = self.load_chunk(arguments[0], offset, chunk)?;
                    self.builder.ins().bnot(value)
                }
                WideOperation::And => {
                    let left = self.load_chunk(arguments[0], offset, chunk)?;
                    let right = self.load_chunk(arguments[1], offset, chunk)?;
                    self.builder.ins().band(left, right)
                }
                WideOperation::Or => {
                    let left = self.load_chunk(arguments[0], offset, chunk)?;
                    let right = self.load_chunk(arguments[1], offset, chunk)?;
                    self.builder.ins().bor(left, right)
                }
                WideOperation::Xor => {
                    let left = self.load_chunk(arguments[0], offset, chunk)?;
                    let right = self.load_chunk(arguments[1], offset, chunk)?;
                    self.builder.ins().bxor(left, right)
                }
                WideOperation::Add => {
                    let left = self.load_chunk(arguments[0], offset, chunk)?;
                    let right = self.load_chunk(arguments[1], offset, chunk)?;
                    let (sum, overflow) = self.builder.ins().uadd_overflow(left, right);
                    let (sum, carry_overflow) = self.builder.ins().uadd_overflow(sum, carry);
                    let overflowed = self.builder.ins().bor(overflow, carry_overflow);
                    carry = self.builder.ins().uextend(types::I64, overflowed);
                    sum
                }
                WideOperation::Sub => {
                    let left = self.load_chunk(arguments[0], offset, chunk)?;
                    let right = self.load_chunk(arguments[1], offset, chunk)?;
                    let (difference, underflow) = self.builder.ins().usub_overflow(left, right);
                    let (difference, borrow_underflow) =
                        self.builder.ins().usub_overflow(difference, carry);
                    let underflowed = self.builder.ins().bor(underflow, borrow_underflow);
                    carry = self.builder.ins().uextend(types::I64, underflowed);
                    difference
                }
            };
            let masked = self.mask(word, chunk);
            let pointer = self.builder.ins().iadd_imm_u(destination, offset as i64);
            self.store_bits(pointer, masked, chunk)?;
            offset += chunk;
        }
        Ok(Lowered::Slot(self.builder.ins().stack_addr(
            self.pointer_type,
            slot,
            0,
        )))
    }

    fn load_chunk(
        &mut self,
        lowered: Lowered,
        offset: usize,
        width: usize,
    ) -> Result<CraneliftValue, CodegenError> {
        match lowered {
            Lowered::Slot(address) => {
                let shifted = self.builder.ins().ishl_imm_u(address, 3);
                let pointer = self.builder.ins().iadd_imm_u(shifted, offset as i64);
                self.load_bits(pointer, width)
            }
            Lowered::Word(word) => {
                let shifted = if offset == 0 {
                    word
                } else {
                    self.builder.ins().ushr_imm_u(word, offset as i64)
                };
                Ok(self.mask(shifted, width))
            }
        }
    }

    fn lower_collection(
        &mut self,
        elements: &[Value],
        expected: &Type,
    ) -> Result<Lowered, CodegenError> {
        let element_types = match expected {
            Type::Collection(types) if types.len() == elements.len() => types,
            _ => return Err(CodegenError::new("a collection has the wrong type")),
        };
        let width = expected.width();
        if width <= 64 {
            let mut word = self.builder.ins().iconst(types::I64, 0);
            let mut offset = 0;
            for (element, element_type) in elements.iter().zip(element_types) {
                let element_word = self.lower_word(element, element_type.width())?;
                let shifted = if offset == 0 {
                    element_word
                } else {
                    self.builder.ins().ishl_imm_u(element_word, offset as i64)
                };
                word = self.builder.ins().bor(word, shifted);
                offset += element_type.width();
            }
            Ok(Lowered::Word(self.mask(word, width)))
        } else {
            let slot = self.allocate_value_slot(width)?;
            let destination = self.slot_bit_pointer(slot);
            let mut offset = 0;
            for (element, element_type) in elements.iter().zip(element_types) {
                let lowered = self.lower_value(element, element_type)?;
                let pointer = self.builder.ins().iadd_imm_u(destination, offset as i64);
                self.write_bits(pointer, lowered, element_type.width())?;
                offset += element_type.width();
            }
            Ok(Lowered::Slot(self.builder.ins().stack_addr(
                self.pointer_type,
                slot,
                0,
            )))
        }
    }

    fn lower_element(
        &mut self,
        operand: &Value,
        index: usize,
        expected: &Type,
    ) -> Result<Lowered, CodegenError> {
        let operand_type = self.type_of(operand)?;
        let (offset, element_type) = element_at(&operand_type, index)?;
        let width = element_type.width();
        if width != expected.width() {
            return Err(CodegenError::new("an element has the wrong width"));
        }
        let lowered = self.lower_value(operand, &operand_type)?;
        match lowered {
            Lowered::Word(word) => {
                let shifted = self.builder.ins().ushr_imm_u(word, offset as i64);
                Ok(Lowered::Word(self.mask(shifted, width)))
            }
            Lowered::Slot(address) => {
                let shifted = self.builder.ins().ishl_imm_u(address, 3);
                let pointer = self.builder.ins().iadd_imm_u(shifted, offset as i64);
                if width <= 64 {
                    Ok(Lowered::Word(self.load_bits(pointer, width)?))
                } else {
                    let slot = self.allocate_value_slot(width)?;
                    let destination = self.slot_bit_pointer(slot);
                    self.copy_bits(pointer, destination, width)?;
                    Ok(Lowered::Slot(self.builder.ins().stack_addr(
                        self.pointer_type,
                        slot,
                        0,
                    )))
                }
            }
        }
    }

    fn lower_reference(
        &mut self,
        slot: Slot,
        offset: usize,
        expected: &Type,
    ) -> Result<Lowered, CodegenError> {
        if !expected.is_reference() {
            return Err(CodegenError::new("a reference has the wrong type"));
        }
        let slot_type = self.slot_type(slot)?;
        let storage = self.storage(slot)?;
        let pointed_type = self.storage_type(slot)?;
        self.type_at_bit_offset(&pointed_type, offset)?;
        let pointer = if slot_type.is_reference() {
            self.load_word(storage)
        } else {
            self.slot_bit_pointer(storage)
        };
        let pointer = if offset == 0 {
            pointer
        } else {
            self.builder.ins().iadd_imm_u(pointer, offset as i64)
        };
        Ok(Lowered::Word(pointer))
    }

    fn lower_load(&mut self, reference: &Value, expected: &Type) -> Result<Lowered, CodegenError> {
        let reference_type = self.type_of(reference)?;
        let inner = match reference_type {
            Type::Reference(inner) => *inner,
            _ => return Err(CodegenError::new("a load needs a reference")),
        };
        if inner.width() != expected.width() {
            return Err(CodegenError::new("a load has the wrong width"));
        }
        let pointer = self.lower_word(reference, 64)?;
        if inner.width() <= 64 {
            Ok(Lowered::Word(self.load_bits(pointer, inner.width())?))
        } else {
            let slot = self.allocate_value_slot(inner.width())?;
            let destination = self.slot_bit_pointer(slot);
            self.copy_bits(pointer, destination, inner.width())?;
            Ok(Lowered::Slot(self.builder.ins().stack_addr(
                self.pointer_type,
                slot,
                0,
            )))
        }
    }

    fn lower_call(
        &mut self,
        label: &str,
        arguments: &[Value],
        expected: &Type,
    ) -> Result<Lowered, CodegenError> {
        let symbol = self
            .symbols
            .functions
            .get(label)
            .ok_or_else(|| CodegenError::new("a callee is not defined"))?;
        let callee = symbol.function;
        let identifier = symbol.identifier;
        if callee.parameters().len() != arguments.len() {
            return Err(CodegenError::new("a call has the wrong arity"));
        }
        if callee.result().width() != expected.width() {
            return Err(CodegenError::new("a call has the wrong width"));
        }
        let mut parameter_types = Vec::new();
        for parameter in callee.parameters() {
            let type_ = callee
                .slots()
                .get(parameter.slot().index())
                .ok_or_else(|| CodegenError::new("a parameter has no slot"))?;
            parameter_types.push(type_.clone());
        }
        let result_width = callee.result().width();
        let mut words = Vec::new();
        let result_slot = if result_width > 64 {
            let slot = self.allocate_value_slot(result_width)?;
            words.push(self.builder.ins().stack_addr(self.pointer_type, slot, 0));
            Some(slot)
        } else {
            None
        };
        for (argument, parameter_type) in arguments.iter().zip(&parameter_types) {
            let lowered = self.lower_value(argument, parameter_type)?;
            match lowered {
                Lowered::Word(word) => words.push(word),
                Lowered::Slot(address) => {
                    let width = parameter_type.width();
                    let temporary = self.allocate_value_slot(width)?;
                    let destination = self.slot_bit_pointer(temporary);
                    let source = self.builder.ins().ishl_imm_u(address, 3);
                    self.copy_bits(source, destination, width)?;
                    let pointer = self
                        .builder
                        .ins()
                        .stack_addr(self.pointer_type, temporary, 0);
                    words.push(pointer);
                }
            }
        }
        let function_reference = self
            .module
            .declare_func_in_func(identifier, self.builder.func);
        let call = self.builder.ins().call(function_reference, &words);
        if let Some(slot) = result_slot {
            Ok(Lowered::Slot(self.builder.ins().stack_addr(
                self.pointer_type,
                slot,
                0,
            )))
        } else {
            Ok(Lowered::Word(self.builder.inst_results(call)[0]))
        }
    }

    fn lower_self_tail_call(
        &mut self,
        label: &str,
        arguments: &[Value],
        expected: &Type,
    ) -> Result<bool, CodegenError> {
        if label != self.function.label().text() {
            return Ok(false);
        }
        if expected.width() != self.function.result().width() {
            return Ok(false);
        }
        let parameters = self.function.parameters().to_vec();
        if parameters.len() != arguments.len() {
            return Ok(false);
        }
        let mut parameter_types = Vec::new();
        for parameter in &parameters {
            parameter_types.push(self.slot_type(parameter.slot())?.clone());
        }
        for argument in arguments {
            if !self.tail_argument_is_safe(argument) {
                return Ok(false);
            }
        }
        let mut lowered = Vec::new();
        for (argument, parameter_type) in arguments.iter().zip(&parameter_types) {
            lowered.push(self.lower_value(argument, parameter_type)?);
        }
        for (parameter, (value, parameter_type)) in
            parameters.iter().zip(lowered.iter().zip(&parameter_types))
        {
            let storage = self.storage(parameter.slot())?;
            self.write_slot(storage, *value, parameter_type.width())?;
        }
        let body_block = self
            .body_block
            .ok_or_else(|| CodegenError::new("a tail call needs a body"))?;
        self.builder.ins().jump(body_block, &[]);
        Ok(true)
    }

    fn tail_argument_is_safe(&self, argument: &Value) -> bool {
        match argument {
            Value::Reference { slot, .. } => self.slot_is_reference_parameter(*slot),
            _ => true,
        }
    }

    fn slot_is_reference_parameter(&self, slot: Slot) -> bool {
        self.function
            .parameters()
            .iter()
            .any(|parameter| parameter.slot() == slot)
            && self
                .slot_type(slot)
                .map(|type_| type_.is_reference())
                .unwrap_or(false)
    }

    fn lower_extern(
        &mut self,
        operation: Operation,
        arguments: &[Value],
        expected: &Type,
    ) -> Result<Lowered, CodegenError> {
        let specification = lookup_operation(operation.code())
            .ok_or_else(|| CodegenError::new("an operation is unknown"))?;
        if specification.argument_widths().len() != arguments.len() {
            return Err(CodegenError::new("an operation has the wrong arity"));
        }
        let payload_width: usize = specification.payload_widths().iter().sum();
        if payload_width > 64 {
            return Err(CodegenError::new("an operation has a wide payload"));
        }
        let result_type = extern_result_type(specification);
        if result_type.width() != expected.width() {
            return Err(CodegenError::new("an operation has the wrong width"));
        }
        let mut words = Vec::new();
        for (argument, width) in arguments.iter().zip(specification.argument_widths()) {
            words.push(self.lower_word(argument, *width)?);
        }
        while words.len() < 4 {
            words.push(self.builder.ins().iconst(types::I64, 0));
        }
        let output_slot = self.allocate_stack_slot(EXTERN_OUTPUT_SIZE)?;
        let output_address = self
            .builder
            .ins()
            .stack_addr(self.pointer_type, output_slot, 0);
        let operation_word = self
            .builder
            .ins()
            .iconst(types::I64, i64::from(operation.code()));
        let function_reference = self
            .module
            .declare_func_in_func(self.symbols.extern_identifier, self.builder.func);
        self.builder.ins().call(
            function_reference,
            &[
                operation_word,
                words[0],
                words[1],
                words[2],
                words[3],
                output_address,
            ],
        );
        let status = self
            .builder
            .ins()
            .load(types::I64, memory_flags(), output_address, 0);
        let payload = self
            .builder
            .ins()
            .load(types::I64, memory_flags(), output_address, 8);
        let total = 1 + payload_width;
        if total <= 64 {
            let shifted = self.builder.ins().ishl_imm_u(payload, 1);
            let word = self.builder.ins().bor(status, shifted);
            Ok(Lowered::Word(self.mask(word, total)))
        } else {
            let slot = self.allocate_value_slot(total)?;
            let destination = self.slot_bit_pointer(slot);
            self.store_bits(destination, status, 1)?;
            let payload_pointer = self.builder.ins().iadd_imm_u(destination, 1);
            self.store_bits(payload_pointer, payload, payload_width)?;
            Ok(Lowered::Slot(self.builder.ins().stack_addr(
                self.pointer_type,
                slot,
                0,
            )))
        }
    }

    fn lower_branch(
        &mut self,
        condition: &Value,
        then: &Block,
        otherwise: &Block,
        expected: &Type,
        tail: bool,
    ) -> Result<Option<Lowered>, CodegenError> {
        let condition_word = self.lower_word(condition, 1)?;
        let result_slot = self.allocate_value_slot(expected.width())?;
        let then_block = self.builder.create_block();
        let else_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(condition_word, then_block, &[], else_block, &[]);
        self.builder.switch_to_block(then_block);
        self.builder.seal_block(then_block);
        let then_result = self.lower_block(then, expected, tail)?;
        if let Some(then_result) = then_result {
            self.write_slot(result_slot, then_result, expected.width())?;
            self.builder.ins().jump(merge_block, &[]);
        }
        self.builder.switch_to_block(else_block);
        self.builder.seal_block(else_block);
        let else_result = self.lower_block(otherwise, expected, tail)?;
        if let Some(else_result) = else_result {
            self.write_slot(result_slot, else_result, expected.width())?;
            self.builder.ins().jump(merge_block, &[]);
        }
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        if then_result.is_none() && else_result.is_none() {
            self.builder.ins().trap(TrapCode::unwrap_user(1));
            return Ok(None);
        }
        Ok(Some(self.read_slot(result_slot, expected)?))
    }

    fn write_bits(
        &mut self,
        pointer: CraneliftValue,
        lowered: Lowered,
        width: usize,
    ) -> Result<(), CodegenError> {
        match lowered {
            Lowered::Word(word) => self.store_bits(pointer, word, width),
            Lowered::Slot(address) => {
                let source = self.builder.ins().ishl_imm_u(address, 3);
                self.copy_bits(source, pointer, width)
            }
        }
    }

    fn write_slot(
        &mut self,
        slot: StackSlot,
        lowered: Lowered,
        width: usize,
    ) -> Result<(), CodegenError> {
        let destination = self.slot_bit_pointer(slot);
        self.write_bits(destination, lowered, width)
    }

    fn read_slot(&mut self, slot: StackSlot, expected: &Type) -> Result<Lowered, CodegenError> {
        if expected.width() <= 64 {
            let word = self.load_word(slot);
            Ok(Lowered::Word(self.mask(word, expected.width())))
        } else {
            Ok(Lowered::Slot(self.builder.ins().stack_addr(
                self.pointer_type,
                slot,
                0,
            )))
        }
    }

    fn load_bits(
        &mut self,
        pointer: CraneliftValue,
        width: usize,
    ) -> Result<CraneliftValue, CodegenError> {
        if width == 0 {
            return Ok(self.builder.ins().iconst(types::I64, 0));
        }
        if width > 64 {
            return Err(CodegenError::new("a word load is too wide"));
        }
        let byte_pointer = self.builder.ins().ushr_imm_u(pointer, 3);
        let shift = self.builder.ins().band_imm_u(pointer, 7);
        let low = self
            .builder
            .ins()
            .load(types::I64, memory_flags(), byte_pointer, 0);
        let high = self
            .builder
            .ins()
            .uload8(types::I64, memory_flags(), byte_pointer, 8);
        let low_shifted = self.builder.ins().ushr(low, shift);
        let negated = self.builder.ins().ineg(shift);
        let high_shifted = self.builder.ins().ishl(high, negated);
        let combined = self.builder.ins().bor(low_shifted, high_shifted);
        let aligned = self.builder.ins().icmp_imm_u(IntCC::Equal, shift, 0);
        let selected = self.builder.ins().select(aligned, low, combined);
        Ok(self.mask(selected, width))
    }

    fn store_bits(
        &mut self,
        pointer: CraneliftValue,
        word: CraneliftValue,
        width: usize,
    ) -> Result<(), CodegenError> {
        if width == 0 {
            return Ok(());
        }
        if width > 64 {
            return Err(CodegenError::new("a word store is too wide"));
        }
        let byte_pointer = self.builder.ins().ushr_imm_u(pointer, 3);
        let shift = self.builder.ins().band_imm_u(pointer, 7);
        let low = self
            .builder
            .ins()
            .load(types::I64, memory_flags(), byte_pointer, 0);
        let low_mask_constant = self.builder.ins().iconst(types::I64, mask_immediate(width));
        let low_mask = self.builder.ins().ishl(low_mask_constant, shift);
        let inverted_low_mask = self.builder.ins().bnot(low_mask);
        let cleared_low = self.builder.ins().band(low, inverted_low_mask);
        let inserted_low = self.builder.ins().ishl(word, shift);
        let new_low = self.builder.ins().bor(cleared_low, inserted_low);
        self.builder
            .ins()
            .store(memory_flags(), new_low, byte_pointer, 0);
        let high = self
            .builder
            .ins()
            .uload8(types::I64, memory_flags(), byte_pointer, 8);
        let extra = self.builder.ins().iadd_imm_s(shift, width as i64 - 64);
        let positive = self
            .builder
            .ins()
            .icmp_imm_s(IntCC::SignedGreaterThan, extra, 0);
        let zero = self.builder.ins().iconst(types::I64, 0);
        let clamped = self.builder.ins().select(positive, extra, zero);
        let one = self.builder.ins().iconst(types::I64, 1);
        let shifted_one = self.builder.ins().ishl(one, clamped);
        let high_mask = self.builder.ins().isub(shifted_one, one);
        let negated = self.builder.ins().ineg(shift);
        let shifted_word = self.builder.ins().ushr(word, negated);
        let inserted_high = self.builder.ins().band(shifted_word, high_mask);
        let inverted_high_mask = self.builder.ins().bnot(high_mask);
        let cleared_high = self.builder.ins().band(high, inverted_high_mask);
        let new_high = self.builder.ins().bor(cleared_high, inserted_high);
        self.builder
            .ins()
            .istore8(memory_flags(), new_high, byte_pointer, 8);
        Ok(())
    }

    fn copy_bits(
        &mut self,
        source: CraneliftValue,
        destination: CraneliftValue,
        width: usize,
    ) -> Result<(), CodegenError> {
        let chunk_count = width / 64;
        let remainder = width % 64;
        if chunk_count > 0 {
            let loop_block = self.builder.create_block();
            let body_block = self.builder.create_block();
            let done_block = self.builder.create_block();
            self.builder.append_block_param(loop_block, types::I64);
            let zero = self.builder.ins().iconst(types::I64, 0);
            self.builder
                .ins()
                .jump(loop_block, &[BlockArg::Value(zero)]);
            self.builder.switch_to_block(loop_block);
            let index = self.builder.block_params(loop_block)[0];
            let condition =
                self.builder
                    .ins()
                    .icmp_imm_u(IntCC::UnsignedLessThan, index, chunk_count as i64);
            self.builder
                .ins()
                .brif(condition, body_block, &[], done_block, &[]);
            self.builder.switch_to_block(body_block);
            self.builder.seal_block(body_block);
            let offset = self.builder.ins().ishl_imm_u(index, 6);
            let source_pointer = self.builder.ins().iadd(source, offset);
            let source_chunk = self.load_bits(source_pointer, 64)?;
            let destination_pointer = self.builder.ins().iadd(destination, offset);
            self.store_bits(destination_pointer, source_chunk, 64)?;
            let next = self.builder.ins().iadd_imm_u(index, 1);
            self.builder
                .ins()
                .jump(loop_block, &[BlockArg::Value(next)]);
            self.builder.seal_block(loop_block);
            self.builder.switch_to_block(done_block);
            self.builder.seal_block(done_block);
        }
        if remainder > 0 {
            let offset = (chunk_count * 64) as i64;
            let source_pointer = self.builder.ins().iadd_imm_u(source, offset);
            let source_chunk = self.load_bits(source_pointer, remainder)?;
            let destination_pointer = self.builder.ins().iadd_imm_u(destination, offset);
            self.store_bits(destination_pointer, source_chunk, remainder)?;
        }
        Ok(())
    }

    fn mask(&mut self, word: CraneliftValue, width: usize) -> CraneliftValue {
        if width >= 64 {
            word
        } else {
            self.builder.ins().band_imm_u(word, mask_immediate(width))
        }
    }

    fn slot_bit_pointer(&mut self, slot: StackSlot) -> CraneliftValue {
        let address = self.builder.ins().stack_addr(self.pointer_type, slot, 0);
        self.builder.ins().ishl_imm_u(address, 3)
    }

    fn store_word(&mut self, slot: StackSlot, word: CraneliftValue) {
        let address = self.builder.ins().stack_addr(self.pointer_type, slot, 0);
        self.builder.ins().store(memory_flags(), word, address, 0);
    }

    fn load_word(&mut self, slot: StackSlot) -> CraneliftValue {
        let address = self.builder.ins().stack_addr(self.pointer_type, slot, 0);
        self.builder
            .ins()
            .load(types::I64, memory_flags(), address, 0)
    }

    fn allocate_stack_slot(&mut self, size: usize) -> Result<StackSlot, CodegenError> {
        create_stack_slot(self.builder, size)
    }

    fn allocate_value_slot(&mut self, width: usize) -> Result<StackSlot, CodegenError> {
        self.allocate_stack_slot(byte_size(width) + STACK_SLOT_SLACK)
    }

    fn storage(&self, slot: Slot) -> Result<StackSlot, CodegenError> {
        self.slots
            .get(slot.index())
            .copied()
            .ok_or_else(|| CodegenError::new("a slot is out of range"))
    }

    fn slot_type(&self, slot: Slot) -> Result<&'program Type, CodegenError> {
        let function = self.function;
        function
            .slots()
            .get(slot.index())
            .ok_or_else(|| CodegenError::new("a slot is out of range"))
    }

    fn storage_type(&self, slot: Slot) -> Result<Type, CodegenError> {
        match self.slot_type(slot)? {
            Type::Reference(inner) => Ok((**inner).clone()),
            other => Ok(other.clone()),
        }
    }

    fn type_of(&self, value: &Value) -> Result<Type, CodegenError> {
        match value {
            Value::Constant(bits) => Ok(flat_type(bits.width())),
            Value::Nand(left, _) => self.type_of(left),
            Value::Intrinsic(_, arguments) => match arguments.first() {
                Some(argument) => self.type_of(argument),
                None => Err(CodegenError::new("an intrinsic has no arguments")),
            },
            Value::Collection(elements) => {
                let mut types = Vec::new();
                for element in elements {
                    types.push(self.type_of(element)?);
                }
                Ok(Type::Collection(types))
            }
            Value::Element { collection, index } => {
                let operand_type = self.type_of(collection)?;
                Ok(element_at(&operand_type, *index)?.1)
            }
            Value::Reference { slot, bit_offset } => {
                let storage = self.storage_type(*slot)?;
                Ok(Type::Reference(Box::new(
                    self.type_at_bit_offset(&storage, *bit_offset)?,
                )))
            }
            Value::Load(reference) => self.accessed_type(reference),
            Value::Call(label, _) => Ok(self.callee(label.text())?.result().clone()),
            Value::Extern(operation, _) => {
                let specification = lookup_operation(operation.code())
                    .ok_or_else(|| CodegenError::new("an operation is unknown"))?;
                Ok(extern_result_type(specification))
            }
            Value::Branch(_, then, otherwise) => {
                if let Some(result) = then.result() {
                    return self.type_of(result);
                }
                if let Some(result) = otherwise.result() {
                    return self.type_of(result);
                }
                Ok(Type::Collection(Vec::new()))
            }
        }
    }

    fn accessed_type(&self, reference: &Value) -> Result<Type, CodegenError> {
        match self.type_of(reference)? {
            Type::Reference(inner) => Ok(*inner),
            _ => Err(CodegenError::new("a load needs a reference")),
        }
    }

    fn type_at_bit_offset(&self, type_: &Type, offset: usize) -> Result<Type, CodegenError> {
        if offset == 0 {
            return Ok(type_.clone());
        }
        match type_ {
            Type::Collection(elements) => {
                let mut remaining = offset;
                for element in elements {
                    let width = element.width();
                    if remaining < width {
                        return self.type_at_bit_offset(element, remaining);
                    }
                    remaining -= width;
                }
                Err(CodegenError::new("a bit offset is out of range"))
            }
            _ => Err(CodegenError::new("a bit offset is out of range")),
        }
    }

    fn callee(&self, label: &str) -> Result<&'program Function, CodegenError> {
        self.symbols
            .functions
            .get(label)
            .map(|symbol| symbol.function)
            .ok_or_else(|| CodegenError::new("a callee is not defined"))
    }
}

fn element_at(type_: &Type, index: usize) -> Result<(usize, Type), CodegenError> {
    let elements = type_
        .elements()
        .ok_or_else(|| CodegenError::new("an element needs a collection"))?;
    let element = elements
        .get(index)
        .ok_or_else(|| CodegenError::new("an element is out of range"))?;
    let offset = elements.iter().take(index).map(Type::width).sum();
    Ok((offset, element.clone()))
}

fn flat_type(width: usize) -> Type {
    if width == 1 {
        Type::Bit
    } else {
        Type::Collection(vec![Type::Bit; width])
    }
}

fn byte_size(width: usize) -> usize {
    width.div_ceil(8)
}

fn create_stack_slot(
    builder: &mut FunctionBuilder,
    size: usize,
) -> Result<StackSlot, CodegenError> {
    let size = u32::try_from(size).map_err(|_| CodegenError::new("a value is too large"))?;
    Ok(builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        size,
        STACK_SLOT_ALIGNMENT_SHIFT,
    )))
}

fn mask_immediate(width: usize) -> i64 {
    if width >= 64 {
        -1
    } else {
        ((1u64 << width) - 1) as i64
    }
}

fn bits_to_word(bits: &[bool], offset: usize, width: usize) -> i64 {
    let mut word = 0u64;
    for index in 0..width {
        if bits[offset + index] {
            word |= 1u64 << index;
        }
    }
    word as i64
}

fn extern_result_type(specification: &OperationSpecification) -> Type {
    let mut elements = vec![Type::Bit];
    for width in specification.payload_widths() {
        elements.push(flat_type(*width));
    }
    Type::Collection(elements)
}

fn export_name(label: &str) -> String {
    if label == "main" {
        "lap_main".to_string()
    } else {
        label.to_string()
    }
}

fn function_linkage(label: &str) -> Linkage {
    if label == "main" {
        Linkage::Export
    } else {
        Linkage::Local
    }
}

fn memory_flags() -> MemFlagsData {
    MemFlagsData::new().with_endianness(Endianness::Little)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift_jit::{JITBuilder, JITModule};
    use lapc_ast::Label;
    use lapc_ir::Parameter;

    fn bit(value: bool) -> Value {
        Value::Constant(BitVector::new(vec![value]))
    }

    fn bits(values: &[bool]) -> Value {
        Value::Constant(BitVector::new(values.to_vec()))
    }

    fn function(
        label: &str,
        parameters: Vec<Parameter>,
        result: Type,
        slots: Vec<Type>,
        body: Block,
    ) -> Function {
        Function::new(Label::new(label), parameters, result, slots, body)
    }

    fn block(statements: Vec<Statement>, result: Value) -> Block {
        Block::new(statements, Some(Box::new(result)))
    }

    fn empty_block(statements: Vec<Statement>) -> Block {
        Block::new(statements, None)
    }

    fn constant_program(value: bool) -> Program {
        Program::new(vec![function(
            "main",
            vec![],
            Type::Bit,
            vec![],
            block(vec![], bit(value)),
        )])
    }

    extern "C" fn test_extern(
        _operation: u64,
        _argument0: u64,
        _argument1: u64,
        _argument2: u64,
        _argument3: u64,
        out: *mut u64,
    ) {
        unsafe {
            *out = 1;
            *out.add(1) = 42;
        }
    }

    fn compile_for_execution(program: &Program) -> (JITModule, HashMap<String, FuncId>) {
        let mut builder = JITBuilder::new(default_libcall_names()).expect("the host is supported");
        builder.symbol("lap_extern", test_extern as *const u8);
        let mut module = JITModule::new(builder);
        let identifiers = lower_program(&mut module, program).expect("the program lowers");
        module
            .finalize_definitions()
            .expect("the functions finalize");
        (module, identifiers)
    }

    fn entry(module: &JITModule, identifiers: &HashMap<String, FuncId>, label: &str) -> *const u8 {
        module.get_finalized_function(identifiers[label])
    }

    fn run_word(program: &Program, label: &str) -> i64 {
        let (module, identifiers) = compile_for_execution(program);
        let function: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, label)) };
        function()
    }

    fn compile_source(source: &str) -> Program {
        let program = lapc_parse::parse_program(source).expect("the source parses");
        lapc_check::check_program(&program).expect("the program checks")
    }

    const WIDE_BUFFER_BYTES: usize = 256;

    fn pattern_bits(width: usize, seed: usize) -> Vec<bool> {
        (0..width).map(|index| (index * 7 + seed) % 5 < 2).collect()
    }

    fn packed_bytes(values: &[bool]) -> Vec<u8> {
        let mut bytes = vec![0u8; values.len().div_ceil(8)];
        for (index, value) in values.iter().enumerate() {
            if *value {
                bytes[index / 8] |= 1 << (index % 8);
            }
        }
        bytes
    }

    fn wide_input(values: &[bool]) -> Vec<u8> {
        let mut bytes = packed_bytes(values);
        bytes.resize(bytes.len() + 8, 0);
        bytes
    }

    fn run_wide(program: &Program, label: &str) -> Vec<u8> {
        let (module, identifiers) = compile_for_execution(program);
        let function: extern "C" fn(*mut u8) =
            unsafe { std::mem::transmute(entry(&module, &identifiers, label)) };
        let mut buffer = [0u8; WIDE_BUFFER_BYTES];
        function(buffer.as_mut_ptr());
        buffer.to_vec()
    }

    fn run_wide_one(program: &Program, label: &str, argument: &[u8]) -> Vec<u8> {
        let (module, identifiers) = compile_for_execution(program);
        let function: extern "C" fn(*mut u8, *const u8) =
            unsafe { std::mem::transmute(entry(&module, &identifiers, label)) };
        let mut buffer = [0u8; WIDE_BUFFER_BYTES];
        function(buffer.as_mut_ptr(), argument.as_ptr());
        buffer.to_vec()
    }

    fn run_wide_two(program: &Program, label: &str, first: &[u8], second: &[u8]) -> Vec<u8> {
        let (module, identifiers) = compile_for_execution(program);
        let function: extern "C" fn(*mut u8, *const u8, *const u8) =
            unsafe { std::mem::transmute(entry(&module, &identifiers, label)) };
        let mut buffer = [0u8; WIDE_BUFFER_BYTES];
        function(buffer.as_mut_ptr(), first.as_ptr(), second.as_ptr());
        buffer.to_vec()
    }

    fn nand_word(left: u128, right: u128, width: usize) -> u128 {
        !(left & right) & mask_wide(width)
    }

    fn expected_intrinsic(intrinsic: Intrinsic, arguments: &[Vec<bool>]) -> Vec<bool> {
        let left = arguments[0].as_slice();
        let right = arguments.get(1).map(Vec::as_slice).unwrap_or(&[]);
        match intrinsic {
            Intrinsic::Not => left.iter().map(|value| !value).collect(),
            Intrinsic::And => left
                .iter()
                .zip(right)
                .map(|(left, right)| left & right)
                .collect(),
            Intrinsic::Or => left
                .iter()
                .zip(right)
                .map(|(left, right)| left | right)
                .collect(),
            Intrinsic::Xor => left
                .iter()
                .zip(right)
                .map(|(left, right)| left ^ right)
                .collect(),
            Intrinsic::Add => add_values(left, right),
            Intrinsic::Sub => sub_values(left, right),
            Intrinsic::Mul => mul_values(left, right),
            Intrinsic::Inc => add_values(left, &bits_of_value(1, left.len())),
            Intrinsic::Dec => sub_values(left, &bits_of_value(1, left.len())),
            Intrinsic::ShiftLeftOne => shift_left(left, 1),
            Intrinsic::ShiftRightOne => shift_right(left, 1),
            Intrinsic::Eq => vec![left == right],
            Intrinsic::Lt => vec![unsigned_less_than(left, right)],
            Intrinsic::IsZero => vec![left.iter().all(|bit| !bit)],
            Intrinsic::Select => {
                let when_one = arguments[1].as_slice();
                let when_zero = arguments[2].as_slice();
                left.iter()
                    .zip(when_one)
                    .zip(when_zero)
                    .map(|((flag, one), zero)| if *flag { *one } else { *zero })
                    .collect()
            }
        }
    }

    fn expected_nand(left: &[bool], right: &[bool]) -> Vec<bool> {
        left.iter()
            .zip(right)
            .map(|(left, right)| !(left & right))
            .collect()
    }

    fn add_values(left: &[bool], right: &[bool]) -> Vec<bool> {
        let mut result = Vec::with_capacity(left.len());
        let mut carry = 0u8;
        for (left, right) in left.iter().zip(right) {
            let sum = u8::from(*left) + u8::from(*right) + carry;
            result.push(sum & 1 == 1);
            carry = sum >> 1;
        }
        result
    }

    fn sub_values(left: &[bool], right: &[bool]) -> Vec<bool> {
        let mut result = Vec::with_capacity(left.len());
        let mut borrow = 0i8;
        for (left, right) in left.iter().zip(right) {
            let difference = i8::from(*left) - i8::from(*right) - borrow;
            result.push(difference & 1 == 1);
            borrow = i8::from(difference < 0);
        }
        result
    }

    fn mul_values(left: &[bool], right: &[bool]) -> Vec<bool> {
        let mut result = vec![false; left.len()];
        for (index, bit) in right.iter().enumerate() {
            if *bit {
                result = add_values(&result, &shift_left(left, index));
            }
        }
        result
    }

    fn shift_left(values: &[bool], amount: usize) -> Vec<bool> {
        (0..values.len())
            .map(|index| index >= amount && values[index - amount])
            .collect()
    }

    fn shift_right(values: &[bool], amount: usize) -> Vec<bool> {
        (0..values.len())
            .map(|index| index + amount < values.len() && values[index + amount])
            .collect()
    }

    fn unsigned_less_than(left: &[bool], right: &[bool]) -> bool {
        for (left, right) in left.iter().zip(right).rev() {
            if left != right {
                return !*left;
            }
        }
        false
    }

    fn mask_wide(width: usize) -> u128 {
        if width >= 128 {
            u128::MAX
        } else {
            (1u128 << width) - 1
        }
    }

    fn value_of(values: &[bool]) -> u128 {
        let mut value = 0u128;
        for (index, bit) in values.iter().enumerate() {
            if *bit {
                value |= 1u128 << index;
            }
        }
        value
    }

    fn bits_of_value(value: u128, width: usize) -> Vec<bool> {
        (0..width).map(|index| (value >> index) & 1 == 1).collect()
    }

    fn constant_function(label: &str, result: Type, value: Value) -> Function {
        function(label, vec![], result, vec![], block(vec![], value))
    }

    fn assert_wide_result(program: &Program, label: &str, values: &[bool]) {
        let expected = packed_bytes(values);
        let wide = run_wide(program, label);
        assert_eq!(&wide[..expected.len()], &expected[..], "for {label}");
    }

    #[test]
    fn jit_runs_an_element_of_mixed_widths() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nmain = () BIT { both: [[BIT, BIT], BIT] = [[BIT.ZERO, BIT.ONE], BIT.ZERO]\n [pair, flag] = both\n flag }\n";
        let program = compile_source(source);
        assert_eq!(run_word(&program, "main"), 0);
    }

    #[test]
    fn emit_object_returns_bytes() {
        let program = constant_program(true);
        let object = emit_object(&program).expect("the object emits");
        assert!(!object.is_empty());
    }

    #[test]
    fn jit_runs_a_constant() {
        let program = constant_program(true);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        assert_eq!(main(), 1);
    }

    #[test]
    fn jit_runs_nand() {
        let value = Value::Nand(Box::new(bit(true)), Box::new(bit(false)));
        let program = Program::new(vec![function(
            "main",
            vec![],
            Type::Bit,
            vec![],
            block(vec![], value),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        assert_eq!(main(), 1);
    }

    #[test]
    fn jit_runs_nand_of_zeros() {
        let value = Value::Nand(Box::new(bit(false)), Box::new(bit(false)));
        let program = Program::new(vec![function(
            "main",
            vec![],
            Type::Bit,
            vec![],
            block(vec![], value),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        assert_eq!(main(), 1);
    }

    const ENTRY: &str = "unsafe extern \"C\" { fn lap_main() -> u64; }\n#[unsafe(no_mangle)]\npub extern \"C\" fn lap_extern(_operation: u64, _argument0: u64, _argument1: u64, _argument2: u64, _argument3: u64, out: *mut u64) {\n    unsafe { *out = 1; *out.add(1) = 42; }\n}\nfn main() { std::process::exit((unsafe { lap_main() } & 1) as i32); }\n";

    fn link_and_run(name: &str, object: &[u8], entry: &str) -> i32 {
        let directory =
            std::env::temp_dir().join(format!("lapc-codegen-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the directory is created");
        let object_path = directory.join("program.o");
        let entry_path = directory.join("entry.rs");
        let binary = directory.join(format!("program{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&object_path, object).expect("the object is written");
        std::fs::write(&entry_path, entry).expect("the entry is written");
        let mut command = std::process::Command::new("rustc");
        command
            .arg("--edition=2024")
            .arg("-o")
            .arg(&binary)
            .arg(&entry_path);
        if cfg!(target_os = "linux") {
            command.arg("-C").arg("relocation-model=static");
        }
        let status = command
            .arg("-C")
            .arg(format!("link-arg={}", object_path.display()))
            .status()
            .expect("the compiler runs");
        assert!(status.success(), "the object links");
        std::process::Command::new(&binary)
            .status()
            .expect("the program runs")
            .code()
            .expect("the program exits with a status")
    }

    fn run_object(program: &Program, name: &str) -> i32 {
        let object = emit_object(program).expect("the object emits");
        link_and_run(name, &object, ENTRY)
    }

    #[test]
    fn the_emitted_object_runs() {
        let value = Value::Nand(Box::new(bit(false)), Box::new(bit(false)));
        let program = Program::new(vec![function(
            "main",
            vec![],
            Type::Bit,
            vec![],
            block(vec![], value),
        )]);
        assert_eq!(run_object(&program, "nand"), 1);
    }

    #[test]
    fn the_emitted_object_runs_a_binding() {
        let value = Value::Load(Box::new(Value::Reference {
            slot: Slot::new(0),
            bit_offset: 0,
        }));
        let program = Program::new(vec![function(
            "main",
            vec![],
            Type::Bit,
            vec![Type::Bit],
            Block::new(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: bit(true),
                }],
                Some(Box::new(value)),
            ),
        )]);
        assert_eq!(run_object(&program, "binding"), 1);
    }

    #[test]
    fn jit_runs_intrinsics() {
        let result = Type::Collection(vec![Type::Bit; 8]);
        let value = Value::Intrinsic(
            Intrinsic::Add,
            vec![
                bits(&[true, false, false, false, false, false, false, false]),
                bits(&[false, true, false, false, false, false, false, false]),
            ],
        );
        let program = Program::new(vec![function(
            "add",
            vec![],
            result,
            vec![],
            block(vec![], value),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let add: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "add")) };
        assert_eq!(add(), 3);
    }

    #[test]
    fn jit_runs_collection_and_element() {
        let collection = Value::Collection(vec![bit(true), bit(false), bit(true)]);
        let value = Value::Element {
            collection: Box::new(collection),
            index: 2,
        };
        let program = Program::new(vec![function(
            "main",
            vec![],
            Type::Bit,
            vec![],
            block(vec![], value),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        assert_eq!(main(), 1);
    }

    #[test]
    fn jit_runs_reference_and_store() {
        let toggle = function(
            "toggle",
            vec![Parameter::new(Slot::new(0))],
            Type::Collection(vec![]),
            vec![Type::Reference(Box::new(Type::Bit))],
            empty_block(vec![Statement::Store {
                reference: Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                },
                value: bit(true),
            }]),
        );
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![Type::Bit, Type::Collection(vec![])],
            block(
                vec![
                    Statement::Bind {
                        slot: Slot::new(0),
                        value: bit(false),
                    },
                    Statement::Bind {
                        slot: Slot::new(1),
                        value: Value::Call(
                            Label::new("toggle"),
                            vec![Value::Reference {
                                slot: Slot::new(0),
                                bit_offset: 0,
                            }],
                        ),
                    },
                ],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        let program = Program::new(vec![toggle, main]);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        assert_eq!(main(), 1);
    }

    #[test]
    fn jit_runs_a_checked_program() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nbit.not = (a: BIT) BIT { NAND(a, a) }\nbit.toggle = (target: *BIT) [] { target = bit.not(target) }\nmain = () BIT { state: BIT = BIT.ZERO\n bit.toggle(*state)\n state }\n";
        let program = compile_source(source);
        assert_eq!(run_word(&program, "main"), 1);
    }

    #[test]
    fn jit_runs_branch() {
        let value = Value::Branch(
            Box::new(bit(true)),
            block(vec![], bit(false)),
            block(vec![], bit(true)),
        );
        let program = Program::new(vec![function(
            "main",
            vec![],
            Type::Bit,
            vec![],
            block(vec![], value),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        assert_eq!(main(), 0);
    }

    #[test]
    fn jit_runs_extern() {
        let value = Value::Extern(Operation::new(0x0400), vec![]);
        let result = Type::Collection(vec![Type::Bit, Type::Collection(vec![Type::Bit; 8])]);
        let program = Program::new(vec![function(
            "main",
            vec![],
            result,
            vec![],
            block(vec![], value),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        assert_eq!(main(), 85);
    }

    #[test]
    fn jit_runs_a_wide_result() {
        let mut values = vec![false; 65];
        values[64] = true;
        let result = Type::Collection(vec![Type::Bit; 65]);
        let program = Program::new(vec![function(
            "wide",
            vec![],
            result,
            vec![],
            block(vec![], bits(&values)),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let wide: extern "C" fn(*mut u8) =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "wide")) };
        let mut buffer = [0u8; 32];
        wide(buffer.as_mut_ptr());
        assert_eq!(buffer[0], 0);
        assert_eq!(buffer[8], 1);
    }

    #[test]
    fn jit_runs_a_wide_parameter() {
        let wide = Type::Collection(vec![Type::Bit; 65]);
        let identity = function(
            "identity",
            vec![Parameter::new(Slot::new(0))],
            wide.clone(),
            vec![wide.clone()],
            block(
                vec![],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        let program = Program::new(vec![identity]);
        let (module, identifiers) = compile_for_execution(&program);
        let identity: extern "C" fn(*mut u8, *const u8) =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "identity")) };
        let mut input = [0u8; 32];
        input[8] = 1;
        let mut output = [0u8; 32];
        identity(output.as_mut_ptr(), input.as_ptr());
        assert_eq!(output[0], 0);
        assert_eq!(output[8], 1);
    }

    #[test]
    fn jit_runs_a_wide_add() {
        let mut left = vec![false; 65];
        left[64] = true;
        let mut right = vec![false; 65];
        right[0] = true;
        let value = Value::Intrinsic(Intrinsic::Add, vec![bits(&left), bits(&right)]);
        let result = Type::Collection(vec![Type::Bit; 65]);
        let program = Program::new(vec![function(
            "add",
            vec![],
            result,
            vec![],
            block(vec![], value),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let add: extern "C" fn(*mut u8) =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "add")) };
        let mut buffer = [0u8; 32];
        add(buffer.as_mut_ptr());
        assert_eq!(buffer[0], 1);
        assert_eq!(buffer[8], 1);
    }

    #[test]
    fn jit_runs_a_wide_element() {
        let mut values = vec![false; 65];
        values[64] = true;
        let value = Value::Element {
            collection: Box::new(bits(&values)),
            index: 64,
        };
        let program = Program::new(vec![function(
            "main",
            vec![],
            Type::Bit,
            vec![],
            block(vec![], value),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        assert_eq!(main(), 1);
    }

    #[test]
    fn jit_runs_a_wide_sub() {
        let mut left = vec![false; 65];
        left[64] = true;
        let mut right = vec![false; 65];
        right[0] = true;
        let value = Value::Intrinsic(Intrinsic::Sub, vec![bits(&left), bits(&right)]);
        let result = Type::Collection(vec![Type::Bit; 65]);
        let program = Program::new(vec![function(
            "sub",
            vec![],
            result,
            vec![],
            block(vec![], value),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let sub: extern "C" fn(*mut u8) =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "sub")) };
        let mut buffer = [0u8; 32];
        sub(buffer.as_mut_ptr());
        assert_eq!(buffer[0], 0xff);
        assert_eq!(buffer[8], 0);
    }

    #[test]
    fn jit_runs_extern_with_a_wide_result() {
        let value = Value::Extern(Operation::new(0x0100), vec![bits(&[false; 64])]);
        let result = Type::Collection(vec![Type::Bit, Type::Collection(vec![Type::Bit; 64])]);
        let program = Program::new(vec![function(
            "main",
            vec![],
            result,
            vec![],
            block(vec![], value),
        )]);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn(*mut u8) =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        let mut buffer = [0u8; 32];
        main(buffer.as_mut_ptr());
        assert_eq!(buffer[0], 85);
        assert_eq!(buffer[8], 0);
    }

    #[test]
    fn jit_runs_a_reference_to_an_element() {
        let pair = Type::Collection(vec![Type::Bit, Type::Bit]);
        let set_middle = function(
            "set_middle",
            vec![Parameter::new(Slot::new(0))],
            Type::Collection(vec![]),
            vec![Type::Reference(Box::new(pair.clone()))],
            empty_block(vec![Statement::Store {
                reference: Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                },
                value: Value::Collection(vec![bit(true), bit(false)]),
            }]),
        );
        let bundle = Type::Collection(vec![Type::Bit, pair.clone()]);
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![
                bundle.clone(),
                Type::Reference(Box::new(pair.clone())),
                Type::Collection(vec![]),
            ],
            block(
                vec![
                    Statement::Bind {
                        slot: Slot::new(0),
                        value: Value::Collection(vec![
                            bit(false),
                            Value::Collection(vec![bit(false), bit(false)]),
                        ]),
                    },
                    Statement::Bind {
                        slot: Slot::new(1),
                        value: Value::Reference {
                            slot: Slot::new(0),
                            bit_offset: 1,
                        },
                    },
                    Statement::Bind {
                        slot: Slot::new(2),
                        value: Value::Call(
                            Label::new("set_middle"),
                            vec![Value::Reference {
                                slot: Slot::new(1),
                                bit_offset: 0,
                            }],
                        ),
                    },
                ],
                Value::Element {
                    collection: Box::new(Value::Element {
                        collection: Box::new(Value::Load(Box::new(Value::Reference {
                            slot: Slot::new(0),
                            bit_offset: 0,
                        }))),
                        index: 1,
                    }),
                    index: 0,
                },
            ),
        );
        let program = Program::new(vec![set_middle, main]);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        assert_eq!(main(), 1);
    }

    #[test]
    fn jit_runs_a_wide_binding() {
        let wide = Type::Collection(vec![Type::Bit; 65]);
        let mut values = vec![false; 65];
        values[64] = true;
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![wide.clone()],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: bits(&values),
                }],
                Value::Element {
                    collection: Box::new(Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: 0,
                    }))),
                    index: 64,
                },
            ),
        );
        let program = Program::new(vec![main]);
        let (module, identifiers) = compile_for_execution(&program);
        let main: extern "C" fn() -> i64 =
            unsafe { std::mem::transmute(entry(&module, &identifiers, "main")) };
        assert_eq!(main(), 1);
    }

    #[test]
    fn emit_object_for_a_program_with_every_construct() {
        let helper = function(
            "helper",
            vec![Parameter::new(Slot::new(0))],
            Type::Bit,
            vec![Type::Reference(Box::new(Type::Bit))],
            block(
                vec![Statement::Store {
                    reference: Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: 0,
                    },
                    value: bit(true),
                }],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![
                Type::Bit,
                Type::Bit,
                Type::Collection(vec![Type::Bit, Type::Bit]),
            ],
            block(
                vec![
                    Statement::Bind {
                        slot: Slot::new(0),
                        value: bit(false),
                    },
                    Statement::Bind {
                        slot: Slot::new(1),
                        value: Value::Call(
                            Label::new("helper"),
                            vec![Value::Reference {
                                slot: Slot::new(0),
                                bit_offset: 0,
                            }],
                        ),
                    },
                    Statement::Bind {
                        slot: Slot::new(2),
                        value: Value::Collection(vec![
                            bit(true),
                            Value::Nand(Box::new(bit(true)), Box::new(bit(true))),
                        ]),
                    },
                ],
                Value::Branch(
                    Box::new(Value::Element {
                        collection: Box::new(Value::Load(Box::new(Value::Reference {
                            slot: Slot::new(2),
                            bit_offset: 0,
                        }))),
                        index: 0,
                    }),
                    block(vec![], Value::Intrinsic(Intrinsic::Not, vec![bit(false)])),
                    block(
                        vec![],
                        Value::Element {
                            collection: Box::new(Value::Extern(Operation::new(0x0400), vec![])),
                            index: 0,
                        },
                    ),
                ),
            ),
        );
        let program = Program::new(vec![helper, main]);
        let object = emit_object(&program).expect("the object emits");
        assert!(!object.is_empty());
        assert_eq!(link_and_run("every-construct", &object, ENTRY), 1);
    }

    #[test]
    fn jit_runs_constants_at_every_width_boundary() {
        for width in [0usize, 1, 2, 7, 8, 63, 64, 65, 127, 128, 129, 192, 1000] {
            let values = pattern_bits(width, 1);
            let program = Program::new(vec![constant_function(
                "constant",
                flat_type(width),
                bits(&values),
            )]);
            if width <= 64 {
                assert_eq!(
                    run_word(&program, "constant"),
                    bits_to_word(&values, 0, width),
                    "at width {width}"
                );
            } else {
                assert_wide_result(&program, "constant", &values);
            }
        }
    }

    #[test]
    fn jit_keeps_every_bit_at_width_64() {
        let mut values = pattern_bits(64, 2);
        values[63] = true;
        let program = Program::new(vec![function(
            "main",
            vec![],
            flat_type(64),
            vec![flat_type(64)],
            Block::new(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: bits(&values),
                }],
                Some(Box::new(Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })))),
            ),
        )]);
        assert_eq!(run_word(&program, "main"), bits_to_word(&values, 0, 64));
    }

    #[test]
    fn jit_masks_a_word_at_width_64() {
        let invert = Program::new(vec![constant_function(
            "operation",
            flat_type(64),
            Value::Intrinsic(Intrinsic::Not, vec![bits(&[false; 64])]),
        )]);
        assert_eq!(run_word(&invert, "operation"), -1);
        let high_bit = bits_of_value(1u128 << 63, 64);
        let wrap = Program::new(vec![constant_function(
            "operation",
            flat_type(64),
            Value::Intrinsic(Intrinsic::Add, vec![bits(&high_bit), bits(&high_bit)]),
        )]);
        assert_eq!(run_word(&wrap, "operation"), 0);
        let borrow = Program::new(vec![constant_function(
            "operation",
            flat_type(64),
            Value::Intrinsic(
                Intrinsic::Sub,
                vec![bits(&[false; 64]), bits(&bits_of_value(1, 64))],
            ),
        )]);
        assert_eq!(run_word(&borrow, "operation"), -1);
    }

    #[test]
    fn jit_runs_nand_at_every_width_boundary() {
        for width in [0usize, 1, 2, 7, 8, 63, 64, 65, 127, 128, 129, 192, 1000] {
            let left = pattern_bits(width, 1);
            let right = pattern_bits(width, 3);
            let expected = expected_nand(&left, &right);
            let value = Value::Nand(Box::new(bits(&left)), Box::new(bits(&right)));
            let program = Program::new(vec![constant_function("nand", flat_type(width), value)]);
            if width <= 64 {
                assert_eq!(
                    run_word(&program, "nand"),
                    bits_to_word(&expected, 0, width),
                    "at width {width}"
                );
            } else {
                assert_wide_result(&program, "nand", &expected);
            }
        }
    }

    #[test]
    fn jit_runs_every_intrinsic_at_every_width_boundary() {
        let intrinsics = [
            Intrinsic::Not,
            Intrinsic::And,
            Intrinsic::Or,
            Intrinsic::Xor,
            Intrinsic::Add,
            Intrinsic::Sub,
        ];
        for width in [0usize, 1, 2, 8, 63, 64, 65, 128, 192, 1000] {
            for intrinsic in intrinsics {
                let left = pattern_bits(width, 2);
                let right = pattern_bits(width, 4);
                let argument_bits = if intrinsic == Intrinsic::Not {
                    vec![left.clone()]
                } else {
                    vec![left.clone(), right.clone()]
                };
                let arguments: Vec<Value> = argument_bits.iter().map(|bits_| bits(bits_)).collect();
                let program = Program::new(vec![constant_function(
                    "intrinsic",
                    flat_type(width),
                    Value::Intrinsic(intrinsic, arguments),
                )]);
                let expected = expected_intrinsic(intrinsic, &argument_bits);
                if width <= 64 {
                    assert_eq!(
                        run_word(&program, "intrinsic"),
                        bits_to_word(&expected, 0, width),
                        "for {intrinsic:?} at width {width}"
                    );
                } else {
                    assert_wide_result(&program, "intrinsic", &expected);
                }
            }
        }
    }

    #[test]
    fn jit_runs_every_new_intrinsic() {
        let intrinsics = [
            Intrinsic::Mul,
            Intrinsic::Inc,
            Intrinsic::Dec,
            Intrinsic::ShiftLeftOne,
            Intrinsic::ShiftRightOne,
            Intrinsic::Eq,
            Intrinsic::Lt,
            Intrinsic::IsZero,
        ];
        for width in [1usize, 2, 4, 8, 16, 32, 63, 64] {
            for intrinsic in intrinsics {
                let left = pattern_bits(width, 2);
                let right = pattern_bits(width, 4);
                let argument_bits = match intrinsic.arity() {
                    1 => vec![left.clone()],
                    _ => vec![left.clone(), right.clone()],
                };
                let arguments: Vec<Value> = argument_bits.iter().map(|bits_| bits(bits_)).collect();
                let result = if intrinsic.is_comparison() {
                    Type::Bit
                } else {
                    flat_type(width)
                };
                let program = Program::new(vec![constant_function(
                    "intrinsic",
                    result,
                    Value::Intrinsic(intrinsic, arguments),
                )]);
                let expected = expected_intrinsic(intrinsic, &argument_bits);
                assert_eq!(
                    run_word(&program, "intrinsic"),
                    bits_to_word(&expected, 0, expected.len()),
                    "for {intrinsic:?} at width {width}"
                );
            }
        }
    }

    #[test]
    fn jit_runs_select() {
        for (flag, when_one, when_zero) in [
            (false, false, false),
            (false, true, false),
            (true, false, true),
            (true, true, true),
        ] {
            let value = Value::Intrinsic(
                Intrinsic::Select,
                vec![bit(flag), bit(when_one), bit(when_zero)],
            );
            let program = Program::new(vec![constant_function("select", Type::Bit, value)]);
            let expected = if flag { when_one } else { when_zero };
            assert_eq!(run_word(&program, "select"), i64::from(expected));
        }
    }

    #[test]
    fn jit_runs_nand_of_structured_collections() {
        let type_ = Type::Collection(vec![
            Type::Collection(vec![Type::Bit, Type::Bit]),
            Type::Bit,
        ]);
        let left = Value::Collection(vec![
            Value::Collection(vec![bit(true), bit(false)]),
            bit(true),
        ]);
        let right = Value::Collection(vec![
            Value::Collection(vec![bit(false), bit(true)]),
            bit(true),
        ]);
        let value = Value::Nand(Box::new(left), Box::new(right));
        let program = Program::new(vec![constant_function("nand", type_, value)]);
        assert_eq!(run_word(&program, "nand"), 3);
    }

    #[test]
    fn jit_runs_an_intrinsic_of_structured_collections() {
        let type_ = Type::Collection(vec![
            Type::Collection(vec![Type::Bit, Type::Bit]),
            Type::Bit,
        ]);
        let left = Value::Collection(vec![
            Value::Collection(vec![bit(true), bit(false)]),
            bit(true),
        ]);
        let right = Value::Collection(vec![
            Value::Collection(vec![bit(false), bit(true)]),
            bit(true),
        ]);
        let value = Value::Intrinsic(Intrinsic::Xor, vec![left, right]);
        let program = Program::new(vec![constant_function("xor", type_, value)]);
        assert_eq!(run_word(&program, "xor"), 3);
    }

    #[test]
    fn jit_runs_a_collection_of_structured_elements() {
        let type_ = Type::Collection(vec![
            Type::Collection(vec![
                Type::Bit,
                Type::Collection(vec![Type::Bit, Type::Bit]),
            ]),
            Type::Bit,
        ]);
        let value = Value::Collection(vec![
            Value::Collection(vec![
                bit(true),
                Value::Collection(vec![bit(false), bit(true)]),
            ]),
            bit(true),
        ]);
        let program = Program::new(vec![constant_function("collection", type_, value)]);
        assert_eq!(run_word(&program, "collection"), 13);
    }

    #[test]
    fn jit_runs_every_element_of_a_structured_collection() {
        let type_ = Type::Collection(vec![
            Type::Collection(vec![Type::Bit, Type::Bit]),
            Type::Bit,
            Type::Collection(vec![Type::Bit, Type::Bit, Type::Bit]),
        ]);
        let collection = Value::Collection(vec![
            Value::Collection(vec![bit(true), bit(false)]),
            bit(true),
            Value::Collection(vec![bit(false), bit(true), bit(true)]),
        ]);
        let elements = type_.elements().expect("the type is a collection");
        for (index, expected) in [(0usize, 1i64), (1, 1), (2, 6)] {
            let value = Value::Element {
                collection: Box::new(collection.clone()),
                index,
            };
            let program = Program::new(vec![constant_function(
                "element",
                elements[index].clone(),
                value,
            )]);
            assert_eq!(
                run_word(&program, "element"),
                expected,
                "for element {index}"
            );
        }
    }

    #[test]
    fn jit_runs_a_wide_element_of_a_wide_collection() {
        let wide = flat_type(65);
        let mut values = pattern_bits(65, 6);
        values[64] = true;
        let collection = Value::Collection(vec![bits(&[true, false, true]), bits(&values)]);
        let value = Value::Element {
            collection: Box::new(collection),
            index: 1,
        };
        let program = Program::new(vec![constant_function("element", wide, value)]);
        assert_wide_result(&program, "element", &values);
    }

    #[test]
    fn jit_runs_a_nested_element_of_mixed_widths() {
        let collection = Value::Collection(vec![
            Value::Collection(vec![bit(true), bit(false)]),
            Value::Collection(vec![bit(true), bit(false)]),
        ]);
        let value = Value::Element {
            collection: Box::new(Value::Element {
                collection: Box::new(collection),
                index: 1,
            }),
            index: 1,
        };
        let program = Program::new(vec![constant_function("element", Type::Bit, value)]);
        assert_eq!(run_word(&program, "element"), 0);
    }

    #[test]
    fn jit_runs_a_source_element_after_a_narrow_element() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nmain = () BIT { bundle: [BIT, [BIT, BIT]] = [BIT.ONE, [BIT.ZERO, BIT.ONE]]\n [first, second] = bundle\n [low, high] = second\n high }\n";
        assert_eq!(run_word(&compile_source(source), "main"), 1);
    }

    #[test]
    fn jit_runs_a_source_element_after_a_wide_element() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nmain = () BIT { bundle: [BIT, [BIT, BIT], BIT] = [BIT.ONE, [BIT.ONE, BIT.ONE], BIT.ZERO]\n [first, second, third] = bundle\n third }\n";
        assert_eq!(run_word(&compile_source(source), "main"), 0);
    }

    #[test]
    fn jit_runs_an_erased_program() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nU8 = [BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]\nliblapc.u8.add = (a: U8, b: U8) U8 { [BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO] }\nmain = () BIT { sum: U8 = liblapc.u8.add([BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ZERO], [BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO])\n [b0, b1, b2, b3, b4, b5, b6, b7] = sum\n b7 }\n";
        let program = lapc_erase::erase_intrinsics(compile_source(source));
        assert_eq!(run_word(&program, "main"), 1);
    }

    #[test]
    fn jit_runs_zero_width_values() {
        let empty = Type::Collection(vec![]);
        let producer = constant_function("empty", empty.clone(), Value::Collection(vec![]));
        let caller = function(
            "caller",
            vec![],
            Type::Bit,
            vec![empty.clone()],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: Value::Call(Label::new("empty"), vec![]),
                }],
                bit(true),
            ),
        );
        let program = Program::new(vec![producer.clone(), caller]);
        assert_eq!(run_word(&program, "caller"), 1);
        let program = Program::new(vec![producer]);
        assert_eq!(run_word(&program, "empty"), 0);
    }

    #[test]
    fn jit_runs_a_zero_width_argument() {
        let empty = Type::Collection(vec![]);
        let identity = function(
            "identity",
            vec![Parameter::new(Slot::new(0))],
            empty.clone(),
            vec![empty.clone()],
            block(
                vec![],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        let caller = function(
            "caller",
            vec![],
            Type::Bit,
            vec![empty.clone()],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: Value::Call(Label::new("identity"), vec![Value::Collection(vec![])]),
                }],
                bit(true),
            ),
        );
        let program = Program::new(vec![identity, caller]);
        assert_eq!(run_word(&program, "caller"), 1);
    }

    #[test]
    fn jit_runs_a_reference_to_a_wide_element_at_every_offset() {
        let wide = flat_type(65);
        for prefix in 0usize..8 {
            let mut stored_values = pattern_bits(65, 9);
            stored_values[64] = true;
            let overwritten_values: Vec<bool> = stored_values.iter().map(|value| !value).collect();
            let padding = pattern_bits(prefix, 1);
            let slot_type = Type::Collection(vec![
                Type::Collection(vec![Type::Bit; prefix]),
                wide.clone(),
            ]);
            let value = Value::Collection(vec![bits(&padding), bits(&overwritten_values)]);
            let statements = vec![
                Statement::Bind {
                    slot: Slot::new(0),
                    value,
                },
                Statement::Store {
                    reference: Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: prefix,
                    },
                    value: bits(&stored_values),
                },
            ];
            let result = Value::Load(Box::new(Value::Reference {
                slot: Slot::new(0),
                bit_offset: 0,
            }));
            let program = Program::new(vec![function(
                "probe",
                vec![],
                slot_type.clone(),
                vec![slot_type],
                block(statements, result),
            )]);
            let expected: Vec<bool> = padding
                .iter()
                .chain(stored_values.iter())
                .copied()
                .collect();
            assert_wide_result(&program, "probe", &expected);
        }
    }

    #[test]
    fn jit_runs_a_reference_to_a_word_at_every_offset() {
        for width in [9usize, 63, 64] {
            for prefix in 0usize..8 {
                let stored_values = pattern_bits(width, 4);
                let overwritten_values: Vec<bool> =
                    stored_values.iter().map(|value| !value).collect();
                let padding = pattern_bits(prefix, 3);
                let slot_type = Type::Collection(vec![
                    Type::Collection(vec![Type::Bit; prefix]),
                    flat_type(width),
                ]);
                let value = Value::Collection(vec![bits(&padding), bits(&overwritten_values)]);
                let statements = vec![
                    Statement::Bind {
                        slot: Slot::new(0),
                        value,
                    },
                    Statement::Store {
                        reference: Value::Reference {
                            slot: Slot::new(0),
                            bit_offset: prefix,
                        },
                        value: bits(&stored_values),
                    },
                ];
                let result = Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                }));
                let program = Program::new(vec![function(
                    "probe",
                    vec![],
                    slot_type.clone(),
                    vec![slot_type],
                    block(statements, result),
                )]);
                let expected: Vec<bool> = padding
                    .iter()
                    .chain(stored_values.iter())
                    .copied()
                    .collect();
                if prefix + width <= 64 {
                    assert_eq!(
                        run_word(&program, "probe"),
                        bits_to_word(&expected, 0, prefix + width),
                        "for width {width} at prefix {prefix}"
                    );
                } else {
                    assert_wide_result(&program, "probe", &expected);
                }
            }
        }
    }

    #[test]
    fn jit_runs_a_wide_call_with_two_wide_arguments() {
        let wide = flat_type(65);
        let nand = function(
            "wide_nand",
            vec![Parameter::new(Slot::new(0)), Parameter::new(Slot::new(1))],
            wide.clone(),
            vec![wide.clone(), wide.clone()],
            block(
                vec![],
                Value::Nand(
                    Box::new(Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: 0,
                    }))),
                    Box::new(Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(1),
                        bit_offset: 0,
                    }))),
                ),
            ),
        );
        let program = Program::new(vec![nand]);
        let left = pattern_bits(65, 1);
        let right = pattern_bits(65, 3);
        let output = run_wide_two(
            &program,
            "wide_nand",
            &wide_input(&left),
            &wide_input(&right),
        );
        let expected = bits_of_value(nand_word(value_of(&left), value_of(&right), 65), 65);
        assert_eq!(
            &output[..packed_bytes(&expected).len()],
            &packed_bytes(&expected)[..]
        );
    }

    #[test]
    fn jit_runs_a_wide_call_result_as_a_wide_argument() {
        let wide = flat_type(65);
        let identity = function(
            "identity",
            vec![Parameter::new(Slot::new(0))],
            wide.clone(),
            vec![wide.clone()],
            block(
                vec![],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        let values = pattern_bits(65, 2);
        let wrapper = function(
            "wrapper",
            vec![],
            wide.clone(),
            vec![],
            block(
                vec![],
                Value::Call(
                    Label::new("identity"),
                    vec![Value::Intrinsic(Intrinsic::Not, vec![bits(&values)])],
                ),
            ),
        );
        let program = Program::new(vec![identity, wrapper]);
        let expected = bits_of_value(!value_of(&values) & mask_wide(65), 65);
        assert_wide_result(&program, "wrapper", &expected);
    }

    #[test]
    fn jit_runs_a_wide_call_from_a_wide_parameter() {
        let wide = flat_type(65);
        let identity = function(
            "identity",
            vec![Parameter::new(Slot::new(0))],
            wide.clone(),
            vec![wide.clone()],
            block(
                vec![],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        let forward = function(
            "forward",
            vec![Parameter::new(Slot::new(0))],
            wide.clone(),
            vec![wide.clone()],
            block(
                vec![],
                Value::Call(
                    Label::new("identity"),
                    vec![Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: 0,
                    }))],
                ),
            ),
        );
        let program = Program::new(vec![identity, forward]);
        let mut values = pattern_bits(65, 8);
        values[64] = true;
        let output = run_wide_one(&program, "forward", &wide_input(&values));
        assert_eq!(
            &output[..packed_bytes(&values).len()],
            &packed_bytes(&values)[..]
        );
    }

    #[test]
    fn jit_runs_every_erased_intrinsic() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nU8 = [BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]\nliblapc.bit.not = (a: BIT) BIT { NAND(a, a) }\nliblapc.bit.xor = (a: BIT, b: BIT) BIT { BIT.ZERO }\nliblapc.bit.or = (a: BIT, b: BIT) BIT { BIT.ZERO }\nliblapc.bit.and = (a: BIT, b: BIT) BIT { BIT.ZERO }\nliblapc.u8.sub = (a: U8, b: U8) U8 { [BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO] }\nliblapc.u8.add = (a: U8, b: U8) U8 { [BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO] }\nmain = () BIT { plain: BIT = liblapc.bit.and(liblapc.bit.not(BIT.ZERO), liblapc.bit.or(BIT.ZERO, liblapc.bit.xor(BIT.ONE, BIT.ZERO)))\n difference: U8 = liblapc.u8.sub([BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO], [BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO])\n [b0, b1, b2, b3, b4, b5, b6, b7] = difference\n NAND(plain, b7) }\n";
        let program = lapc_erase::erase_intrinsics(compile_source(source));
        assert_eq!(run_word(&program, "main"), 0);
    }

    #[test]
    fn emit_object_handles_an_empty_program() {
        let object = emit_object(&Program::new(vec![])).expect("the object emits");
        assert!(!object.is_empty());
    }

    #[test]
    fn jit_runs_a_nand_of_wide_elements() {
        let collection = Value::Collection(vec![bits(&pattern_bits(65, 1)), bit(true)]);
        let value = Value::Nand(
            Box::new(Value::Element {
                collection: Box::new(collection.clone()),
                index: 1,
            }),
            Box::new(Value::Element {
                collection: Box::new(collection),
                index: 1,
            }),
        );
        let program = Program::new(vec![constant_function("nand", Type::Bit, value)]);
        assert_eq!(run_word(&program, "nand"), 0);
    }

    #[test]
    fn jit_runs_an_ignored_element() {
        let collection = Value::Collection(vec![bit(true), bit(false)]);
        let program = Program::new(vec![function(
            "main",
            vec![],
            Type::Bit,
            vec![],
            block(
                vec![Statement::Evaluate {
                    value: Value::Element {
                        collection: Box::new(collection),
                        index: 0,
                    },
                }],
                bit(true),
            ),
        )]);
        assert_eq!(run_word(&program, "main"), 1);
    }

    #[test]
    fn jit_runs_an_empty_branch() {
        let empty = Type::Collection(vec![]);
        let value = Value::Branch(
            Box::new(bit(true)),
            empty_block(vec![]),
            empty_block(vec![Statement::Evaluate { value: bit(false) }]),
        );
        let program = Program::new(vec![constant_function("branch", empty, value)]);
        assert_eq!(run_word(&program, "branch"), 0);
    }

    #[test]
    fn jit_runs_a_branch_with_a_wide_result() {
        let wide = flat_type(65);
        for condition in [true, false] {
            let then_values = pattern_bits(65, 1);
            let else_values = pattern_bits(65, 3);
            let value = Value::Branch(
                Box::new(bit(condition)),
                block(vec![], bits(&then_values)),
                block(vec![], bits(&else_values)),
            );
            let program = Program::new(vec![constant_function("branch", wide.clone(), value)]);
            let expected = if condition {
                &then_values
            } else {
                &else_values
            };
            assert_wide_result(&program, "branch", expected);
        }
    }

    #[test]
    fn jit_runs_a_nested_branch_with_a_wide_result() {
        let wide = flat_type(65);
        let inner = Value::Branch(
            Box::new(bit(false)),
            block(vec![], bits(&pattern_bits(65, 1))),
            block(vec![], bits(&pattern_bits(65, 3))),
        );
        let outer = Value::Branch(
            Box::new(bit(true)),
            block(vec![], inner),
            block(vec![], bits(&pattern_bits(65, 5))),
        );
        let program = Program::new(vec![constant_function("branch", wide, outer)]);
        assert_wide_result(&program, "branch", &pattern_bits(65, 3));
    }

    #[test]
    fn jit_runs_a_branch_with_a_wide_binding() {
        let wide = flat_type(65);
        let then_values = pattern_bits(65, 1);
        let program = Program::new(vec![function(
            "branch",
            vec![],
            wide.clone(),
            vec![wide.clone()],
            block(
                vec![],
                Value::Branch(
                    Box::new(bit(true)),
                    Block::new(
                        vec![Statement::Bind {
                            slot: Slot::new(0),
                            value: bits(&then_values),
                        }],
                        Some(Box::new(Value::Load(Box::new(Value::Reference {
                            slot: Slot::new(0),
                            bit_offset: 0,
                        })))),
                    ),
                    Block::new(
                        vec![Statement::Bind {
                            slot: Slot::new(0),
                            value: bits(&pattern_bits(65, 3)),
                        }],
                        Some(Box::new(Value::Load(Box::new(Value::Reference {
                            slot: Slot::new(0),
                            bit_offset: 0,
                        })))),
                    ),
                ),
            ),
        )]);
        assert_wide_result(&program, "branch", &then_values);
    }

    #[test]
    fn jit_runs_extern_at_every_result_width() {
        let cases = [
            (Operation::new(0x0000), vec![bits(&[true; 8])], 0usize),
            (Operation::new(0x0400), vec![], 8),
            (Operation::new(0x0100), vec![bits(&[false; 64])], 64),
            (
                Operation::new(0x0200),
                vec![
                    bits(&[false; 64]),
                    bits(&[false; 64]),
                    bits(&[false; 64]),
                    bits(&[false; 8]),
                ],
                64,
            ),
        ];
        for (operation, arguments, payload_width) in cases {
            let result = extern_result_type(
                lookup_operation(operation.code()).expect("the operation is known"),
            );
            let program = Program::new(vec![constant_function(
                "call",
                result,
                Value::Extern(operation, arguments),
            )]);
            let word = 1u128 | (42u128 << 1);
            if payload_width < 64 {
                assert_eq!(
                    run_word(&program, "call"),
                    (word & mask_wide(1 + payload_width)) as i64,
                    "for payload width {payload_width}"
                );
            } else {
                assert_wide_result(&program, "call", &bits_of_value(word, 1 + payload_width));
            }
        }
    }

    #[test]
    fn jit_runs_an_extern_with_a_structured_argument() {
        let argument = Value::Collection(vec![bits(&pattern_bits(64, 5))]);
        let result = extern_result_type(lookup_operation(0x0101).expect("the operation is known"));
        let value = Value::Extern(Operation::new(0x0101), vec![argument]);
        let program = Program::new(vec![constant_function("call", result, value)]);
        assert_eq!(run_word(&program, "call"), 1);
    }

    #[test]
    fn jit_runs_an_extern_payload_bit() {
        let value = Value::Element {
            collection: Box::new(Value::Element {
                collection: Box::new(Value::Extern(
                    Operation::new(0x0100),
                    vec![bits(&[false; 64])],
                )),
                index: 1,
            }),
            index: 1,
        };
        let program = Program::new(vec![constant_function("call", Type::Bit, value)]);
        assert_eq!(run_word(&program, "call"), 1);
    }

    #[test]
    fn emit_object_rejects_an_element_out_of_range() {
        let value = Value::Element {
            collection: Box::new(Value::Collection(vec![bit(true)])),
            index: 1,
        };
        let program = Program::new(vec![constant_function("main", Type::Bit, value)]);
        let error = emit_object(&program).expect_err("the object is rejected");
        assert_eq!(error.message(), "an element is out of range");
    }

    #[test]
    fn emit_object_rejects_an_element_of_a_non_collection() {
        let value = Value::Element {
            collection: Box::new(bit(true)),
            index: 0,
        };
        let program = Program::new(vec![constant_function("main", Type::Bit, value)]);
        let error = emit_object(&program).expect_err("the object is rejected");
        assert_eq!(error.message(), "an element needs a collection");
    }

    #[test]
    fn emit_object_rejects_a_constant_of_the_wrong_width() {
        let program = Program::new(vec![constant_function(
            "main",
            Type::Bit,
            bits(&[true, false]),
        )]);
        let error = emit_object(&program).expect_err("the object is rejected");
        assert_eq!(error.message(), "a constant has the wrong width");
    }

    #[test]
    fn emit_object_rejects_an_unknown_operation() {
        let value = Value::Extern(Operation::new(0x7fff), vec![]);
        let program = Program::new(vec![constant_function("main", Type::Bit, value)]);
        let error = emit_object(&program).expect_err("the object is rejected");
        assert_eq!(error.message(), "an operation is unknown");
    }

    #[test]
    fn emit_object_rejects_a_reference_of_the_wrong_type() {
        let value = Value::Reference {
            slot: Slot::new(0),
            bit_offset: 0,
        };
        let program = Program::new(vec![function(
            "main",
            vec![],
            Type::Collection(vec![Type::Bit, Type::Bit]),
            vec![Type::Bit],
            block(vec![], value),
        )]);
        let error = emit_object(&program).expect_err("the object is rejected");
        assert_eq!(error.message(), "a reference has the wrong type");
    }

    #[test]
    fn the_emitted_object_runs_a_wide_constant() {
        let wide = flat_type(65);
        let mut values = pattern_bits(65, 1);
        values[64] = true;
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![wide],
            Block::new(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: bits(&values),
                }],
                Some(Box::new(Value::Element {
                    collection: Box::new(Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: 0,
                    }))),
                    index: 64,
                })),
            ),
        );
        assert_eq!(run_object(&Program::new(vec![main]), "wide-constant"), 1);
    }

    #[test]
    fn the_emitted_object_runs_a_wide_branch() {
        let mut true_values = pattern_bits(65, 1);
        true_values[64] = true;
        let mut false_values = pattern_bits(65, 3);
        false_values[64] = false;
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![],
            block(
                vec![],
                Value::Element {
                    collection: Box::new(Value::Branch(
                        Box::new(bit(true)),
                        block(vec![], bits(&true_values)),
                        block(vec![], bits(&false_values)),
                    )),
                    index: 64,
                },
            ),
        );
        assert_eq!(run_object(&Program::new(vec![main]), "wide-branch"), 1);
    }

    #[test]
    fn the_emitted_object_runs_a_wide_call() {
        let wide = flat_type(65);
        let identity = function(
            "identity",
            vec![Parameter::new(Slot::new(0))],
            wide.clone(),
            vec![wide.clone()],
            block(
                vec![],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        let mut values = pattern_bits(65, 2);
        values[64] = true;
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![wide.clone()],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: Value::Call(Label::new("identity"), vec![bits(&values)]),
                }],
                Value::Element {
                    collection: Box::new(Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: 0,
                    }))),
                    index: 64,
                },
            ),
        );
        assert_eq!(
            run_object(&Program::new(vec![identity, main]), "wide-call"),
            1
        );
    }

    #[test]
    fn the_emitted_object_runs_an_extern_with_a_wide_result() {
        let value = Value::Element {
            collection: Box::new(Value::Element {
                collection: Box::new(Value::Extern(
                    Operation::new(0x0100),
                    vec![bits(&[false; 64])],
                )),
                index: 1,
            }),
            index: 1,
        };
        let main = constant_function("main", Type::Bit, value);
        let object = emit_object(&Program::new(vec![main])).expect("the object emits");
        assert_eq!(link_and_run("extern-wide", &object, ENTRY), 1);
    }

    const COUNTING_PRELUDE: &str = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\n\
         BIT.ZERO = NAND(BIT.ONE, BIT.ONE)\n\
         bit.not = (a: BIT) BIT { NAND(a, a) }\n\
         bit.and = (a: BIT, b: BIT) BIT { bit.not(NAND(a, b)) }\n\
         U2 = [BIT, BIT]\n\
         U2.ZERO = [BIT.ZERO, BIT.ZERO]\n\
         U2.ONE = [BIT.ONE, BIT.ZERO]\n\
         U2.TWO = [BIT.ZERO, BIT.ONE]\n\
         U2.THREE = [BIT.ONE, BIT.ONE]\n\
         u2.is.zero = (value: U2) BIT { [low, high] = value\n bit.and(bit.not(low), bit.not(high)) }\n\
         u2.dec = (value: U2) U2 { [low, high] = value\n BRANCH (low) { [BIT.ZERO, high] } { [BIT.ONE, bit.not(high)] } }\n";

    #[test]
    fn jit_runs_a_self_tail_call_as_a_loop() {
        let program = compile_source(&format!(
            "{COUNTING_PRELUDE}\
             count.down = (count: U2) BIT {{ BRANCH (u2.is.zero(count)) {{ BIT.ONE }} {{ count.down(u2.dec(count)) }} }}\n\
             main = () BIT {{ count.down(U2.THREE) }}\n"
        ));
        assert_eq!(run_word(&program, "main"), 1);
    }

    #[test]
    fn jit_keeps_a_tail_call_with_a_local_reference_as_a_call() {
        let program = compile_source(&format!(
            "{COUNTING_PRELUDE}\
             count.down = (count: U2, target: *U2) BIT {{ BRANCH (u2.is.zero(count)) {{ [low, high] = target\n low }} {{ count.down(u2.dec(count), *count) }} }}\n\
             main = () BIT {{ count: U2 = U2.THREE\n count.down(count, *count) }}\n"
        ));
        assert_eq!(run_word(&program, "main"), 1);
    }
}
