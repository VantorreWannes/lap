use std::collections::HashSet;
use std::fmt::Write;

use lapc_ir::{Block, Function, Intrinsic, Program, Slot, Statement, Type, Value};

mod compiler;

pub use compiler::{Compiler, find_compiler};

const PRELUDE: &str = r#"
static uint64_t lap_mask(uint64_t width) {
  return (width >= 64) ? ~(uint64_t)0 : (((uint64_t)1 << width) - 1);
}

static uint64_t lap_load(const uint64_t *base, uint64_t bit, uint64_t width) {
  uint64_t index = bit >> 6;
  uint64_t shift = bit & 63;
  uint64_t low = base[index] >> shift;
  uint64_t high = (shift == 0) ? 0 : (base[index + 1] << (64 - shift));
  return (low | high) & lap_mask(width);
}

static void lap_store(uint64_t *base, uint64_t bit, uint64_t width, uint64_t value) {
  uint64_t index = bit >> 6;
  uint64_t shift = bit & 63;
  uint64_t mask = lap_mask(width);
  uint64_t field = mask << shift;
  base[index] = (base[index] & ~field) | ((value & mask) << shift);
  if (shift != 0 && shift + width > 64) {
    uint64_t extra = shift + width - 64;
    uint64_t high_mask = lap_mask(extra);
    uint64_t high = (value & mask) >> (64 - shift);
    base[index + 1] = (base[index + 1] & ~high_mask) | (high & high_mask);
  }
}

static void lap_copy(uint64_t *destination, const uint64_t *source, uint64_t width) {
  uint64_t words = (width + 63) >> 6;
  for (uint64_t index = 0; index < words; index++) {
    destination[index] = source[index];
  }
}

static void lap_wide_mask(uint64_t *value, uint64_t width) {
  uint64_t words = (width + 63) >> 6;
  if (words > 0) {
    value[words - 1] &= lap_mask(width - (words - 1) * 64);
  }
}

static void lap_wide_not(uint64_t *out, const uint64_t *a, uint64_t width) {
  uint64_t words = (width + 63) >> 6;
  for (uint64_t index = 0; index < words; index++) {
    out[index] = ~a[index];
  }
  lap_wide_mask(out, width);
}

static void lap_wide_and(uint64_t *out, const uint64_t *a, const uint64_t *b, uint64_t width) {
  uint64_t words = (width + 63) >> 6;
  for (uint64_t index = 0; index < words; index++) {
    out[index] = a[index] & b[index];
  }
  lap_wide_mask(out, width);
}

static void lap_wide_or(uint64_t *out, const uint64_t *a, const uint64_t *b, uint64_t width) {
  uint64_t words = (width + 63) >> 6;
  for (uint64_t index = 0; index < words; index++) {
    out[index] = a[index] | b[index];
  }
  lap_wide_mask(out, width);
}

static void lap_wide_xor(uint64_t *out, const uint64_t *a, const uint64_t *b, uint64_t width) {
  uint64_t words = (width + 63) >> 6;
  for (uint64_t index = 0; index < words; index++) {
    out[index] = a[index] ^ b[index];
  }
  lap_wide_mask(out, width);
}

static void lap_wide_nand(uint64_t *out, const uint64_t *a, const uint64_t *b, uint64_t width) {
  uint64_t words = (width + 63) >> 6;
  for (uint64_t index = 0; index < words; index++) {
    out[index] = ~(a[index] & b[index]);
  }
  lap_wide_mask(out, width);
}

static void lap_wide_add(uint64_t *out, const uint64_t *a, const uint64_t *b, uint64_t width) {
  uint64_t words = (width + 63) >> 6;
  uint64_t carry = 0;
  for (uint64_t index = 0; index < words; index++) {
    uint64_t sum = a[index] + b[index];
    uint64_t first = sum < a[index];
    uint64_t total = sum + carry;
    uint64_t second = total < sum;
    carry = first | second;
    out[index] = total;
  }
  lap_wide_mask(out, width);
}

static void lap_wide_sub(uint64_t *out, const uint64_t *a, const uint64_t *b, uint64_t width) {
  uint64_t words = (width + 63) >> 6;
  uint64_t borrow = 0;
  for (uint64_t index = 0; index < words; index++) {
    uint64_t difference = a[index] - b[index];
    uint64_t first = a[index] < b[index];
    uint64_t total = difference - borrow;
    uint64_t second = difference < borrow;
    borrow = first | second;
    out[index] = total;
  }
  lap_wide_mask(out, width);
}
"#;

pub fn emit_c(program: &Program) -> Result<String, CodegenError> {
    let labels = emitted_labels(program);
    let mut output = String::new();
    output.push_str("#include <stdbool.h>\n#include <stdint.h>\n");
    output.push_str(PRELUDE);
    for specification in lapc_extern::OPERATIONS {
        let arguments = "uint64_t, ".repeat(specification.argument_widths().len());
        let _ = writeln!(
            output,
            "extern void {}({}uint64_t *out);",
            operation_symbol(specification),
            arguments
        );
    }
    output.push('\n');
    for function in program.functions() {
        if labels.contains(function.label().text()) {
            let _ = writeln!(output, "{};", declaration(function));
        }
    }
    output.push('\n');
    for function in program.functions() {
        if labels.contains(function.label().text()) {
            output.push_str(&definition(program, function).map_err(CodegenError::new)?);
            output.push('\n');
        }
    }
    Ok(output)
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

fn emitted_labels(program: &Program) -> HashSet<&str> {
    if !program
        .functions()
        .iter()
        .any(|function| function.label().text() == "main")
    {
        return program
            .functions()
            .iter()
            .map(|function| function.label().text())
            .collect();
    }
    let mut reached = HashSet::new();
    let mut pending = vec!["main"];
    while let Some(label) = pending.pop() {
        if !reached.insert(label) {
            continue;
        }
        let Some(function) = program
            .functions()
            .iter()
            .find(|function| function.label().text() == label)
        else {
            continue;
        };
        collect_calls_block(function.body(), &mut pending);
    }
    reached
}

fn collect_calls_block<'program>(block: &'program Block, pending: &mut Vec<&'program str>) {
    for statement in block.statements() {
        match statement {
            Statement::Bind { value, .. } => collect_calls_value(value, pending),
            Statement::Store { reference, value } => {
                collect_calls_value(reference, pending);
                collect_calls_value(value, pending);
            }
            Statement::Evaluate { value } => collect_calls_value(value, pending),
        }
    }
    if let Some(result) = block.result() {
        collect_calls_value(result, pending);
    }
}

fn collect_calls_value<'program>(value: &'program Value, pending: &mut Vec<&'program str>) {
    match value {
        Value::Call(label, arguments) => {
            pending.push(label.text());
            for argument in arguments {
                collect_calls_value(argument, pending);
            }
        }
        Value::Nand(left, right) => {
            collect_calls_value(left, pending);
            collect_calls_value(right, pending);
        }
        Value::Intrinsic(_, arguments)
        | Value::Collection(arguments)
        | Value::Extern(_, arguments) => {
            for argument in arguments {
                collect_calls_value(argument, pending);
            }
        }
        Value::Element { collection, .. } => collect_calls_value(collection, pending),
        Value::Load(reference) => collect_calls_value(reference, pending),
        Value::Branch(condition, then, otherwise) => {
            collect_calls_value(condition, pending);
            collect_calls_block(then, pending);
            collect_calls_block(otherwise, pending);
        }
        Value::Constant(_) | Value::Reference { .. } => {}
    }
}

fn operation_symbol(specification: &lapc_extern::OperationSpecification) -> String {
    format!("lap_extern_{}", specification.name().replace('.', "_"))
}

fn c_name(label: &str) -> String {
    if label == "main" {
        "lap_main".to_string()
    } else {
        format!("lapc_{}", label.replace('.', "_"))
    }
}

fn declaration(function: &Function) -> String {
    let wide = function.result().width() > 64;
    let mut parameters = Vec::new();
    if wide {
        parameters.push("uint64_t *out".to_string());
    }
    for parameter in function.parameters() {
        let index = parameter.slot().index();
        let type_ = &function.slots()[index];
        if type_.is_reference() {
            parameters.push(format!("uint64_t *p{index}"));
            parameters.push(format!("uint64_t pb{index}"));
        } else if type_.width() > 64 {
            parameters.push(format!("const uint64_t *p{index}"));
        } else {
            parameters.push(format!("uint64_t p{index}"));
        }
    }
    let result = if wide { "void" } else { "uint64_t" };
    let storage = if function.label().text() == "main" {
        ""
    } else {
        "static "
    };
    format!(
        "{storage}{result} {}({})",
        c_name(function.label().text()),
        parameters.join(", ")
    )
}

fn definition(program: &Program, function: &Function) -> Result<String, String> {
    let mut emitter = Emitter::new(program, function);
    emitter.emit()?;
    Ok(emitter.output)
}

struct Emitter<'program> {
    program: &'program Program,
    function: &'program Function,
    output: String,
    indent: usize,
    temporaries: usize,
}

enum Emitted {
    Word(String),
    Pointer(String),
    Reference { base: String, bit: String },
}

impl<'program> Emitter<'program> {
    fn new(program: &'program Program, function: &'program Function) -> Self {
        Self {
            program,
            function,
            output: String::new(),
            indent: 1,
            temporaries: 0,
        }
    }

    fn emit(&mut self) -> Result<(), String> {
        let result_type = self.function.result().clone();
        self.line(declaration(self.function));
        self.line("{");
        for (index, type_) in self.function.slots().iter().enumerate() {
            if type_.is_reference() {
                self.line(format!("uint64_t *s{index} = 0;"));
                self.line(format!("uint64_t sb{index} = 0;"));
            } else {
                self.line(format!(
                    "uint64_t s{index}[{}] = {{0}};",
                    words(type_.width())
                ));
            }
        }
        for parameter in self.function.parameters() {
            let index = parameter.slot().index();
            let type_ = self.function.slots()[index].clone();
            if type_.is_reference() {
                self.line(format!("s{index} = p{index};"));
                self.line(format!("sb{index} = pb{index};"));
            } else if type_.width() > 64 {
                self.line(format!("lap_copy(s{index}, p{index}, {});", type_.width()));
            } else {
                self.line(format!(
                    "s{index}[0] = p{index} & lap_mask({});",
                    type_.width()
                ));
            }
        }
        self.line("body:;");
        let result = self.block(self.function.body(), &result_type, true)?;
        match result {
            Some(Emitted::Word(word)) => self.line(format!("return {word};")),
            Some(Emitted::Pointer(pointer)) => {
                self.line(format!(
                    "lap_copy(out, {pointer}, {});",
                    result_type.width()
                ));
                self.line("return;");
            }
            Some(Emitted::Reference { .. }) => {
                return Err("a reference result is not supported".to_string());
            }
            None => {
                if result_type.width() > 64 {
                    self.line("return;");
                } else {
                    self.line("return 0;");
                }
            }
        }
        self.line("}");
        Ok(())
    }

    fn line(&mut self, text: impl AsRef<str>) {
        let indent = "  ".repeat(self.indent);
        let _ = writeln!(self.output, "{indent}{}", text.as_ref());
    }

    fn temporary_word(&mut self) -> String {
        let name = format!("t{}", self.temporaries);
        self.temporaries += 1;
        self.line(format!("uint64_t {name} = 0;"));
        name
    }

    fn temporary_array(&mut self, width: usize) -> String {
        let name = format!("t{}", self.temporaries);
        self.temporaries += 1;
        self.line(format!("uint64_t {name}[{}] = {{0}};", words(width)));
        name
    }

    fn block(
        &mut self,
        block: &Block,
        expected: &Type,
        tail: bool,
    ) -> Result<Option<Emitted>, String> {
        for statement in block.statements() {
            self.statement(statement)?;
        }
        match block.result() {
            Some(value) => self.result(value, expected, tail),
            None => Ok(None),
        }
    }

    fn result(
        &mut self,
        value: &Value,
        expected: &Type,
        tail: bool,
    ) -> Result<Option<Emitted>, String> {
        if tail {
            if let Value::Call(label, arguments) = value {
                if label.text() == self.function.label().text()
                    && self.tail_arguments_are_safe(arguments)
                {
                    self.tail_call(arguments)?;
                    return Ok(None);
                }
            }
        }
        Ok(Some(self.value(value, expected, tail)?))
    }

    fn tail_arguments_are_safe(&self, arguments: &[Value]) -> bool {
        arguments.iter().all(|argument| match argument {
            Value::Reference { slot, .. } => self.slot_is_reference_parameter(*slot),
            _ => true,
        })
    }

    fn slot_is_reference_parameter(&self, slot: Slot) -> bool {
        self.function.slots()[slot.index()].is_reference()
            && self
                .function
                .parameters()
                .iter()
                .any(|parameter| parameter.slot() == slot)
    }

    fn tail_call(&mut self, arguments: &[Value]) -> Result<(), String> {
        let parameters = self.function.parameters().to_vec();
        let mut assignments = Vec::new();
        for (parameter, argument) in parameters.iter().zip(arguments) {
            let index = parameter.slot().index();
            let type_ = self.function.slots()[index].clone();
            if type_.is_reference() {
                let (base, bit) = self.reference_parts(argument)?;
                let base_temporary = self.temporary_pointer();
                let bit_temporary = self.temporary_word();
                self.line(format!("{base_temporary} = {base};"));
                self.line(format!("{bit_temporary} = {bit};"));
                assignments.push(format!("s{index} = {base_temporary};"));
                assignments.push(format!("sb{index} = {bit_temporary};"));
            } else if type_.width() > 64 {
                let pointer = self.pointer(argument, &type_)?;
                let temporary = self.temporary_array(type_.width());
                self.line(format!(
                    "lap_copy({temporary}, {pointer}, {});",
                    type_.width()
                ));
                assignments.push(format!(
                    "lap_copy(s{index}, {temporary}, {});",
                    type_.width()
                ));
            } else {
                let word = self.word(argument, &type_)?;
                let temporary = self.temporary_word();
                self.line(format!("{temporary} = {word};"));
                assignments.push(format!("s{index}[0] = {temporary};"));
            }
        }
        for assignment in assignments {
            self.line(assignment);
        }
        self.line("goto body;");
        Ok(())
    }

    fn temporary_pointer(&mut self) -> String {
        let name = format!("t{}", self.temporaries);
        self.temporaries += 1;
        self.line(format!("uint64_t *{name} = 0;"));
        name
    }

    fn statement(&mut self, statement: &Statement) -> Result<(), String> {
        match statement {
            Statement::Bind { slot, value } => {
                let type_ = self.function.slots()[slot.index()].clone();
                if type_.is_reference() {
                    let (base, bit) = self.reference_parts(value)?;
                    self.line(format!("s{} = {base};", slot.index()));
                    self.line(format!("sb{} = {bit};", slot.index()));
                } else if type_.width() > 64 {
                    let pointer = self.pointer(value, &type_)?;
                    self.line(format!(
                        "lap_copy(s{}, {pointer}, {});",
                        slot.index(),
                        type_.width()
                    ));
                } else {
                    let word = self.word(value, &type_)?;
                    self.line(format!("s{}[0] = {word};", slot.index()));
                }
                Ok(())
            }
            Statement::Store { reference, value } => {
                let (base, bit) = self.reference_parts(reference)?;
                let type_ = self.type_of(value)?;
                if type_.width() > 64 {
                    let pointer = self.pointer(value, &type_)?;
                    for index in 0..type_.width().div_ceil(64) {
                        self.line(format!(
                            "lap_store({base}, ({bit}) + {}, 64, {pointer}[{index}]);",
                            index * 64
                        ));
                    }
                } else {
                    let word = self.word(value, &type_)?;
                    self.line(format!(
                        "lap_store({base}, {bit}, {}, {word});",
                        type_.width()
                    ));
                }
                Ok(())
            }
            Statement::Evaluate { value } => {
                let type_ = self.type_of(value)?;
                if type_.width() > 64 {
                    let pointer = self.pointer(value, &type_)?;
                    self.line(format!("(void){pointer};"));
                } else {
                    let word = self.word(value, &type_)?;
                    self.line(format!("(void)({word});"));
                }
                Ok(())
            }
        }
    }

    fn word(&mut self, value: &Value, expected: &Type) -> Result<String, String> {
        match self.value(value, expected, false)? {
            Emitted::Word(word) => Ok(word),
            Emitted::Pointer(_) | Emitted::Reference { .. } => {
                Err("a word was expected".to_string())
            }
        }
    }

    fn pointer(&mut self, value: &Value, expected: &Type) -> Result<String, String> {
        match self.value(value, expected, false)? {
            Emitted::Pointer(pointer) => Ok(pointer),
            Emitted::Word(_) | Emitted::Reference { .. } => {
                Err("a pointer was expected".to_string())
            }
        }
    }

    fn reference_parts(&mut self, value: &Value) -> Result<(String, String), String> {
        match value {
            Value::Reference { slot, bit_offset } => {
                let index = slot.index();
                if self.function.slots()[index].is_reference() {
                    let bit = if *bit_offset == 0 {
                        format!("sb{index}")
                    } else {
                        format!("(sb{index} + {bit_offset})")
                    };
                    Ok((format!("s{index}"), bit))
                } else {
                    Ok((format!("s{index}"), bit_offset.to_string()))
                }
            }
            _ => Err("a reference was expected".to_string()),
        }
    }

    fn value(&mut self, value: &Value, expected: &Type, tail: bool) -> Result<Emitted, String> {
        if expected.is_reference() {
            let (base, bit) = self.reference_parts(value)?;
            return Ok(Emitted::Reference { base, bit });
        }
        match value {
            Value::Constant(bits) => self.constant(bits.bits(), expected.width()),
            Value::Nand(left, right) => {
                if expected.width() > 64 {
                    let name = self.temporary_array(expected.width());
                    let left_pointer = self.pointer(left, expected)?;
                    let right_pointer = self.pointer(right, expected)?;
                    self.line(format!(
                        "lap_wide_nand({name}, {left_pointer}, {right_pointer}, {});",
                        expected.width()
                    ));
                    return Ok(Emitted::Pointer(name));
                }
                let left_word = self.word(left, expected)?;
                let right_word = self.word(right, expected)?;
                Ok(Emitted::Word(format!(
                    "((~(({left_word}) & ({right_word}))) & lap_mask({}))",
                    expected.width()
                )))
            }
            Value::Intrinsic(intrinsic, arguments) => {
                self.intrinsic(*intrinsic, arguments, expected)
            }
            Value::Collection(elements) => self.collection(elements, expected),
            Value::Element { collection, index } => self.element(collection, *index, expected),
            Value::Reference { .. } => Err("a bare reference is not supported".to_string()),
            Value::Load(reference) => self.load(reference, expected),
            Value::Call(label, arguments) => self.call(label.text(), arguments, expected),
            Value::Extern(operation, arguments) => self.external(*operation, arguments, expected),
            Value::Branch(condition, then, otherwise) => {
                self.branch(condition, then, otherwise, expected, tail)
            }
        }
    }

    fn constant(&mut self, bits: &[bool], width: usize) -> Result<Emitted, String> {
        if width <= 64 {
            return Ok(Emitted::Word(word_literal(bits_to_word(bits, width))));
        }
        let name = self.temporary_array(width);
        let mut offset = 0;
        while offset < width {
            let chunk = (width - offset).min(64);
            let word = bits_to_word(&bits[offset..], chunk);
            self.line(format!("{name}[{}] = {};", offset / 64, word_literal(word)));
            offset += chunk;
        }
        Ok(Emitted::Pointer(name))
    }

    fn intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        arguments: &[Value],
        expected: &Type,
    ) -> Result<Emitted, String> {
        if intrinsic == Intrinsic::DivMod {
            return self.divide(arguments, expected);
        }
        if intrinsic == Intrinsic::AddWithCarry || intrinsic == Intrinsic::SubWithBorrow {
            return self.carry(intrinsic, arguments, expected);
        }
        let width = expected.width();
        if width > 64 {
            let helper = match intrinsic {
                Intrinsic::Not => "lap_wide_not",
                Intrinsic::And => "lap_wide_and",
                Intrinsic::Or => "lap_wide_or",
                Intrinsic::Xor => "lap_wide_xor",
                Intrinsic::Add => "lap_wide_add",
                Intrinsic::Sub => "lap_wide_sub",
                _ => return Err("a wide intrinsic is not supported".to_string()),
            };
            let name = self.temporary_array(width);
            let mut arguments_text = vec![name.clone()];
            for argument in arguments {
                let type_ = self.type_of(argument)?;
                arguments_text.push(self.pointer(argument, &type_)?);
            }
            arguments_text.push(width.to_string());
            self.line(format!("{helper}({});", arguments_text.join(", ")));
            return Ok(Emitted::Pointer(name));
        }
        let mut words = Vec::new();
        for argument in arguments {
            let type_ = self.type_of(argument)?;
            words.push(self.word(argument, &type_)?);
        }
        let expression = match intrinsic {
            Intrinsic::Not => format!("~({})", words[0]),
            Intrinsic::And => format!("({}) & ({})", words[0], words[1]),
            Intrinsic::Or => format!("({}) | ({})", words[0], words[1]),
            Intrinsic::Xor => format!("({}) ^ ({})", words[0], words[1]),
            Intrinsic::Add => format!("({}) + ({})", words[0], words[1]),
            Intrinsic::Sub => format!("({}) - ({})", words[0], words[1]),
            Intrinsic::Mul => format!("({}) * ({})", words[0], words[1]),
            Intrinsic::Inc => format!("({}) + 1", words[0]),
            Intrinsic::Dec => format!("({}) - 1", words[0]),
            Intrinsic::ShiftLeftOne => format!("({}) << 1", words[0]),
            Intrinsic::ShiftRightOne => format!("({}) >> 1", words[0]),
            Intrinsic::Eq => format!("(({}) == ({}))", words[0], words[1]),
            Intrinsic::Lt => format!("(({}) < ({}))", words[0], words[1]),
            Intrinsic::IsZero => format!("(({}) == 0)", words[0]),
            Intrinsic::Select => format!("(({}) ? ({}) : ({}))", words[0], words[1], words[2]),
            Intrinsic::DivMod => {
                return Err("a division is lowered before word operations".to_string());
            }
            Intrinsic::AddWithCarry | Intrinsic::SubWithBorrow => {
                return Err("a carry operation is lowered before word operations".to_string());
            }
        };
        Ok(Emitted::Word(format!(
            "(({expression}) & lap_mask({width}))"
        )))
    }

    fn carry(
        &mut self,
        intrinsic: Intrinsic,
        arguments: &[Value],
        expected: &Type,
    ) -> Result<Emitted, String> {
        if arguments.len() != 3 {
            return Err("a carry operation has the wrong arity".to_string());
        }
        let operand_type = self.type_of(&arguments[0])?;
        let operand_width = operand_type.width();
        if operand_width > 64 {
            return Err("a carry operation is only defined for widths up to 64".to_string());
        }
        let paired = matches!(
            expected,
            Type::Collection(elements)
                if elements.len() == 2
                    && elements[0] == operand_type
                    && elements[1] == Type::Bit
        );
        if !paired {
            return Err("a carry operation has the wrong type".to_string());
        }
        let left = self.word(&arguments[0], &operand_type)?;
        let right = self.word(&arguments[1], &operand_type)?;
        let carry = self.word(&arguments[2], &Type::Bit)?;
        let left_name = self.temporary_word();
        let right_name = self.temporary_word();
        let carry_name = self.temporary_word();
        self.line(format!("{left_name} = {left};"));
        self.line(format!("{right_name} = {right};"));
        self.line(format!("{carry_name} = {carry};"));
        let partial_name = self.temporary_word();
        let first_name = self.temporary_word();
        let total_name = self.temporary_word();
        let second_name = self.temporary_word();
        if intrinsic == Intrinsic::AddWithCarry {
            self.line(format!("{partial_name} = {left_name} + {right_name};"));
            self.line(format!("{first_name} = {partial_name} < {left_name};"));
            self.line(format!("{total_name} = {partial_name} + {carry_name};"));
            self.line(format!("{second_name} = {total_name} < {partial_name};"));
        } else {
            self.line(format!("{partial_name} = {left_name} - {right_name};"));
            self.line(format!("{first_name} = {left_name} < {right_name};"));
            self.line(format!("{total_name} = {partial_name} - {carry_name};"));
            self.line(format!("{second_name} = {partial_name} < {carry_name};"));
        }
        let out = format!("({first_name} | {second_name})");
        let width = expected.width();
        if width <= 64 {
            return Ok(Emitted::Word(format!(
                "((({total_name}) | (({out}) << {operand_width})) & lap_mask({width}))"
            )));
        }
        let name = self.temporary_array(width);
        self.line(format!(
            "lap_store({name}, 0, {operand_width}, {total_name});"
        ));
        self.line(format!("lap_store({name}, {operand_width}, 1, {out});"));
        Ok(Emitted::Pointer(name))
    }

    fn divide(&mut self, arguments: &[Value], expected: &Type) -> Result<Emitted, String> {
        if arguments.len() != 2 {
            return Err("a division has the wrong arity".to_string());
        }
        let operand_type = self.type_of(&arguments[0])?;
        let operand_width = operand_type.width();
        if operand_width > 64 {
            return Err("a division is only defined for widths up to 64".to_string());
        }
        let paired = matches!(
            expected,
            Type::Collection(elements)
                if elements.len() == 2
                    && elements[0] == operand_type
                    && elements[1] == operand_type
        );
        if !paired {
            return Err("a division has the wrong type".to_string());
        }
        let value = self.word(&arguments[0], &operand_type)?;
        let divisor = self.word(&arguments[1], &operand_type)?;
        let value_name = self.temporary_word();
        let divisor_name = self.temporary_word();
        self.line(format!("{value_name} = {value};"));
        self.line(format!("{divisor_name} = {divisor};"));
        let quotient = format!("(({divisor_name}) == 0 ? 0 : (({value_name}) / ({divisor_name})))");
        let remainder =
            format!("(({divisor_name}) == 0 ? 0 : (({value_name}) % ({divisor_name})))");
        let width = expected.width();
        if width <= 64 {
            return Ok(Emitted::Word(format!(
                "((({quotient}) | (({remainder}) << {operand_width})) & lap_mask({width}))"
            )));
        }
        let name = self.temporary_array(width);
        self.line(format!(
            "lap_store({name}, 0, {operand_width}, {quotient});"
        ));
        self.line(format!(
            "lap_store({name}, {operand_width}, {operand_width}, {remainder});"
        ));
        Ok(Emitted::Pointer(name))
    }

    fn collection(&mut self, elements: &[Value], expected: &Type) -> Result<Emitted, String> {
        let types = match expected {
            Type::Collection(types) if types.len() == elements.len() => types.clone(),
            _ => return Err("a collection has the wrong type".to_string()),
        };
        if expected.width() <= 64 {
            let mut word = "0".to_string();
            let mut offset = 0;
            for (element, element_type) in elements.iter().zip(&types) {
                let element_word = self.word(element, element_type)?;
                let shifted = if offset == 0 {
                    element_word
                } else {
                    format!("(({element_word}) << {offset})")
                };
                word = format!("(({word}) | ({shifted}))");
                offset += element_type.width();
            }
            return Ok(Emitted::Word(format!(
                "(({word}) & lap_mask({}))",
                expected.width()
            )));
        }
        let name = self.temporary_array(expected.width());
        let mut offset = 0;
        for (element, element_type) in elements.iter().zip(&types) {
            if element_type.width() > 64 {
                let pointer = self.pointer(element, element_type)?;
                for index in 0..element_type.width().div_ceil(64) {
                    self.line(format!(
                        "lap_store({name}, {}, 64, {pointer}[{index}]);",
                        offset + index * 64
                    ));
                }
            } else {
                let element_word = self.word(element, element_type)?;
                self.line(format!(
                    "lap_store({name}, {offset}, {}, {element_word});",
                    element_type.width()
                ));
            }
            offset += element_type.width();
        }
        Ok(Emitted::Pointer(name))
    }

    fn element(
        &mut self,
        collection: &Value,
        index: usize,
        expected: &Type,
    ) -> Result<Emitted, String> {
        let collection_type = self.type_of(collection)?;
        let (offset, element_type) = element_at(&collection_type, index)?;
        if element_type.width() != expected.width() {
            return Err("an element has the wrong width".to_string());
        }
        if element_type.width() <= 64 {
            return match self.value(collection, &collection_type, false)? {
                Emitted::Word(word) => {
                    let shifted = if offset == 0 {
                        word
                    } else {
                        format!("(({word}) >> {offset})")
                    };
                    Ok(Emitted::Word(format!(
                        "(({shifted}) & lap_mask({}))",
                        element_type.width()
                    )))
                }
                Emitted::Pointer(pointer) => Ok(Emitted::Word(format!(
                    "lap_load({pointer}, {offset}, {})",
                    element_type.width()
                ))),
                Emitted::Reference { .. } => {
                    Err("an element of a reference is not supported".to_string())
                }
            };
        }
        let pointer = self.pointer(collection, &collection_type)?;
        if offset % 64 == 0 {
            return Ok(Emitted::Pointer(format!("({pointer} + {})", offset / 64)));
        }
        let name = self.temporary_array(element_type.width());
        for index in 0..element_type.width().div_ceil(64) {
            self.line(format!(
                "{name}[{index}] = lap_load({pointer}, {}, 64);",
                offset + index * 64
            ));
        }
        Ok(Emitted::Pointer(name))
    }

    fn load(&mut self, reference: &Value, expected: &Type) -> Result<Emitted, String> {
        let (base, bit) = self.reference_parts(reference)?;
        if expected.width() <= 64 {
            return Ok(Emitted::Word(format!(
                "lap_load({base}, {bit}, {})",
                expected.width()
            )));
        }
        let name = self.temporary_array(expected.width());
        for index in 0..expected.width().div_ceil(64) {
            self.line(format!(
                "{name}[{index}] = lap_load({base}, ({bit}) + {}, 64);",
                index * 64
            ));
        }
        Ok(Emitted::Pointer(name))
    }

    fn call(
        &mut self,
        label: &str,
        arguments: &[Value],
        expected: &Type,
    ) -> Result<Emitted, String> {
        let callee = self
            .program
            .functions()
            .iter()
            .find(|function| function.label().text() == label)
            .ok_or_else(|| format!("a callee is not defined: {label}"))?;
        let mut words = Vec::new();
        let wide = callee.result().width() > 64;
        let out = if wide {
            let name = self.temporary_array(expected.width());
            words.push(name);
            true
        } else {
            false
        };
        let parameter_types: Vec<Type> = callee
            .parameters()
            .iter()
            .map(|parameter| callee.slots()[parameter.slot().index()].clone())
            .collect();
        for (argument, parameter_type) in arguments.iter().zip(&parameter_types) {
            if parameter_type.is_reference() {
                let (base, bit) = self.reference_parts(argument)?;
                words.push(base);
                words.push(bit);
            } else if parameter_type.width() > 64 {
                let pointer = self.pointer(argument, parameter_type)?;
                words.push(pointer);
            } else {
                let word = self.word(argument, parameter_type)?;
                words.push(word);
            }
        }
        let call = format!("{}({})", c_name(label), words.join(", "));
        if out {
            self.line(format!("{call};"));
            Ok(Emitted::Pointer(words[0].clone()))
        } else {
            Ok(Emitted::Word(call))
        }
    }

    fn external(
        &mut self,
        operation: lapc_extern::Operation,
        arguments: &[Value],
        expected: &Type,
    ) -> Result<Emitted, String> {
        let specification = lapc_extern::lookup(operation.code())
            .ok_or_else(|| "an operation is unknown".to_string())?;
        let mut words = Vec::new();
        for argument in arguments {
            let type_ = self.type_of(argument)?;
            words.push(self.word(argument, &type_)?);
        }
        let out = self.temporary_array(64);
        self.line(format!("{out}[0] = 0;"));
        self.line(format!("{out}[1] = 0;"));
        words.push(out.clone());
        self.line(format!(
            "{}({});",
            operation_symbol(specification),
            words.join(", ")
        ));
        let payload_width: usize = specification.payload_widths().iter().sum();
        if expected.width() <= 64 {
            return Ok(Emitted::Word(format!(
                "((({out}[0] & 1)) | (({out}[1] & lap_mask({payload_width})) << 1)) & lap_mask({})",
                expected.width()
            )));
        }
        let name = self.temporary_array(expected.width());
        self.line(format!("lap_store({name}, 0, 1, {out}[0] & 1);"));
        self.line(format!("lap_store({name}, 1, {payload_width}, {out}[1]);"));
        Ok(Emitted::Pointer(name))
    }

    fn branch(
        &mut self,
        condition: &Value,
        then: &Block,
        otherwise: &Block,
        expected: &Type,
        tail: bool,
    ) -> Result<Emitted, String> {
        let condition_word = self.word(condition, &Type::Bit)?;
        let target = self.temporary(expected);
        self.line(format!("if ({condition_word}) {{"));
        self.indent += 1;
        if let Some(value) = self.block(then, expected, tail)? {
            self.assign(&target, value, expected)?;
        }
        self.indent -= 1;
        self.line("} else {");
        self.indent += 1;
        if let Some(value) = self.block(otherwise, expected, tail)? {
            self.assign(&target, value, expected)?;
        }
        self.indent -= 1;
        self.line("}");
        Ok(target)
    }

    fn temporary(&mut self, expected: &Type) -> Emitted {
        if expected.is_reference() {
            let base = self.temporary_pointer();
            let bit = self.temporary_word();
            Emitted::Reference { base, bit }
        } else if expected.width() > 64 {
            Emitted::Pointer(self.temporary_array(expected.width()))
        } else {
            Emitted::Word(self.temporary_word())
        }
    }

    fn assign(&mut self, target: &Emitted, value: Emitted, expected: &Type) -> Result<(), String> {
        match (target, value) {
            (Emitted::Word(target), Emitted::Word(value)) => {
                self.line(format!("{target} = {value};"));
                Ok(())
            }
            (Emitted::Pointer(target), Emitted::Pointer(value)) => {
                self.line(format!(
                    "lap_copy({target}, {value}, {});",
                    expected.width()
                ));
                Ok(())
            }
            (
                Emitted::Reference { base, bit },
                Emitted::Reference {
                    base: value_base,
                    bit: value_bit,
                },
            ) => {
                self.line(format!("{base} = {value_base};"));
                self.line(format!("{bit} = {value_bit};"));
                Ok(())
            }
            _ => Err("a branch arm has the wrong shape".to_string()),
        }
    }

    fn type_of(&self, value: &Value) -> Result<Type, String> {
        match value {
            Value::Constant(bits) => Ok(flat_type(bits.width())),
            Value::Nand(left, _) => self.type_of(left),
            Value::Intrinsic(intrinsic, arguments) => {
                if intrinsic.is_comparison() {
                    return Ok(Type::Bit);
                }
                let argument = arguments
                    .first()
                    .ok_or_else(|| "an intrinsic has no arguments".to_string())?;
                let operand = self.type_of(argument)?;
                Ok(intrinsic.result_type(&operand))
            }
            Value::Collection(elements) => {
                let mut types = Vec::new();
                for element in elements {
                    types.push(self.type_of(element)?);
                }
                Ok(Type::Collection(types))
            }
            Value::Element { collection, index } => {
                let collection_type = self.type_of(collection)?;
                Ok(element_at(&collection_type, *index)?.1)
            }
            Value::Reference { slot, bit_offset } => {
                let storage = self.storage_type(*slot)?;
                Ok(Type::Reference(Box::new(type_at_bit_offset(
                    &storage,
                    *bit_offset,
                )?)))
            }
            Value::Load(reference) => match self.type_of(reference)? {
                Type::Reference(inner) => Ok(*inner),
                _ => Err("a load needs a reference".to_string()),
            },
            Value::Call(label, _) => {
                let callee = self
                    .program
                    .functions()
                    .iter()
                    .find(|function| function.label().text() == label.text())
                    .ok_or_else(|| format!("a callee is not defined: {}", label.text()))?;
                Ok(callee.result().clone())
            }
            Value::Extern(operation, _) => {
                let specification = lapc_extern::lookup(operation.code())
                    .ok_or_else(|| "an operation is unknown".to_string())?;
                Ok(lapc_ir::extern_result_type(specification))
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

    fn storage_type(&self, slot: Slot) -> Result<Type, String> {
        match &self.function.slots()[slot.index()] {
            Type::Reference(inner) => Ok((**inner).clone()),
            other => Ok(other.clone()),
        }
    }
}

fn type_at_bit_offset(type_: &Type, offset: usize) -> Result<Type, String> {
    if offset == 0 {
        return Ok(type_.clone());
    }
    match type_ {
        Type::Collection(elements) => {
            let mut remaining = offset;
            for element in elements {
                let width = element.width();
                if remaining < width {
                    return type_at_bit_offset(element, remaining);
                }
                remaining -= width;
            }
            Err("a bit offset is out of range".to_string())
        }
        _ => Err("a bit offset needs a collection".to_string()),
    }
}

fn element_at(type_: &Type, index: usize) -> Result<(usize, Type), String> {
    let elements = type_
        .elements()
        .ok_or_else(|| "an element needs a collection".to_string())?;
    let element = elements
        .get(index)
        .ok_or_else(|| "an element is out of range".to_string())?;
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

fn words(width: usize) -> usize {
    width.div_ceil(64) + 1
}

fn word_literal(word: u64) -> String {
    format!("((uint64_t)0x{word:x})")
}

fn bits_to_word(bits: &[bool], width: usize) -> u64 {
    let mut word = 0u64;
    for (index, bit) in bits.iter().take(width).enumerate() {
        if *bit {
            word |= 1u64 << index;
        }
    }
    word
}

#[cfg(test)]
mod tests {
    use super::{Compiler, bits_to_word, c_name, emit_c, find_compiler, flat_type};

    use std::fs;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use lapc_ast::Label;
    use lapc_ir::{BitVector, Block, Function, Parameter, Program, Slot, Statement, Type, Value};

    const RUNTIME_SOURCE: &str = include_str!("../../../runtime/lap_runtime.c");

    fn compiler() -> Option<&'static Compiler> {
        static COMPILER: OnceLock<Option<Compiler>> = OnceLock::new();
        COMPILER.get_or_init(|| find_compiler().ok()).as_ref()
    }

    static NUMBER: AtomicUsize = AtomicUsize::new(0);

    fn temporary_directory(name: &str) -> PathBuf {
        let number = NUMBER.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "lapc-codegen-{}-{number}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("the test directory is created");
        directory
    }

    enum Argument {
        Word(u64),
        Wide(Vec<u64>),
        Reference { storage: Vec<u64>, bit: u64 },
    }

    fn word_list(words: &[u64]) -> String {
        words
            .iter()
            .map(|word| format!("0x{word:x}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn driver_source(function: &Function, label: &str, arguments: &[Argument]) -> String {
        let mut source = String::from(
            "\n#include <stdio.h>\n#include <stdbool.h>\n#include <stdint.h>\n\
         extern bool lap_runtime_start(int, char **);\n\
         extern void lap_runtime_finish(void);\n",
        );
        let mut words = Vec::new();
        for (index, argument) in arguments.iter().enumerate() {
            let name = format!("a{index}");
            let type_ = &function.slots()[function.parameters()[index].slot().index()];
            match argument {
                Argument::Word(word) => {
                    assert!(
                        !type_.is_reference() && type_.width() <= 64,
                        "a word argument"
                    );
                    words.push(word.to_string());
                }
                Argument::Wide(values) => {
                    assert!(
                        !type_.is_reference() && type_.width() > 64,
                        "a wide argument"
                    );
                    source.push_str(&format!(
                        "static const uint64_t {name}[] = {{{}}};\n",
                        word_list(values)
                    ));
                    words.push(name);
                }
                Argument::Reference { storage, bit } => {
                    assert!(type_.is_reference(), "a reference argument");
                    source.push_str(&format!(
                        "static uint64_t {name}[] = {{{}}};\n",
                        word_list(storage)
                    ));
                    words.push(name);
                    words.push(bit.to_string());
                }
            }
        }
        let wide = function.result().width() > 64;
        if wide {
            source.push_str(&format!(
                "static uint64_t out[{}] = {{0}};\n",
                function.result().width().div_ceil(64)
            ));
            words.insert(0, String::from("out"));
        }
        source.push_str("int main(void) {\n  lap_runtime_start(0, 0);\n");
        let call = format!("{}({})", c_name(label), words.join(", "));
        if wide {
            source.push_str(&format!("  {call};\n"));
            source.push_str(&format!(
                "  fwrite(out, 1, {}, stdout);\n",
                function.result().width().div_ceil(8)
            ));
        } else {
            source.push_str(&format!("  uint64_t result = {call};\n"));
            source.push_str("  fwrite(&result, 1, 8, stdout);\n");
        }
        source.push_str("  lap_runtime_finish();\n  return 0;\n}\n");
        source
    }

    fn run(program: &Program, label: &str, arguments: &[Argument]) -> Option<Vec<u8>> {
        let compiler = compiler()?;
        let function = program
            .functions()
            .iter()
            .find(|function| function.label().text() == label)
            .expect("the callee is defined");
        let mut source = emit_c(program).expect("the program emits");
        source.push_str(&driver_source(function, label, arguments));
        let directory = temporary_directory(label);
        let program_path = directory.join("program.c");
        let runtime_path = directory.join("lap_runtime.c");
        let binary = directory.join(format!("program{}", std::env::consts::EXE_SUFFIX));
        fs::write(&program_path, source).expect("the program is written");
        fs::write(&runtime_path, RUNTIME_SOURCE).expect("the runtime is written");
        let output = compiler
            .command()
            .arg("-std=c17")
            .arg("-O2")
            .arg("-o")
            .arg(&binary)
            .arg(&program_path)
            .arg(&runtime_path)
            .stdout(Stdio::null())
            .output()
            .expect("the C compiler runs");
        assert!(
            output.status.success(),
            "the generated C compiles: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(&binary).output().expect("the program runs");
        assert!(
            output.status.success(),
            "the program exits successfully: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = fs::remove_dir_all(&directory);
        Some(output.stdout)
    }

    fn run_word(program: &Program, label: &str) -> Option<u64> {
        let bytes = run(program, label, &[])?;
        Some(u64::from_le_bytes(
            bytes[..8].try_into().expect("eight bytes"),
        ))
    }

    fn run_word_call(program: &Program, label: &str, arguments: &[Argument]) -> Option<u64> {
        let bytes = run(program, label, arguments)?;
        Some(u64::from_le_bytes(
            bytes[..8].try_into().expect("eight bytes"),
        ))
    }

    fn run_wide(program: &Program, label: &str) -> Option<Vec<u8>> {
        run(program, label, &[])
    }

    fn run_wide_call(program: &Program, label: &str, arguments: &[Argument]) -> Option<Vec<u8>> {
        run(program, label, arguments)
    }

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

    fn constant_function(label: &str, result: Type, value: Value) -> Function {
        function(label, vec![], result, vec![], block(vec![], value))
    }

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

    fn compile_source(source: &str) -> Program {
        let program = lapc_parse::parse_program(source).expect("the source parses");
        let program = lapc_check::check_program(&program).expect("the program checks");
        lapc_erase::erase_intrinsics(program)
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

    fn add_values(left: &[bool], right: &[bool]) -> Vec<bool> {
        add_with_carry_values(left, right, false).0
    }

    fn add_with_carry_values(left: &[bool], right: &[bool], carry: bool) -> (Vec<bool>, bool) {
        let mut result = Vec::with_capacity(left.len());
        let mut carry = u8::from(carry);
        for (left, right) in left.iter().zip(right) {
            let sum = u8::from(*left) + u8::from(*right) + carry;
            result.push(sum & 1 == 1);
            carry = sum >> 1;
        }
        (result, carry == 1)
    }

    fn sub_values(left: &[bool], right: &[bool]) -> Vec<bool> {
        sub_with_borrow_values(left, right, false).0
    }

    fn sub_with_borrow_values(left: &[bool], right: &[bool], borrow: bool) -> (Vec<bool>, bool) {
        let mut result = Vec::with_capacity(left.len());
        let mut borrow = i8::from(borrow);
        for (left, right) in left.iter().zip(right) {
            let difference = i8::from(*left) - i8::from(*right) - borrow;
            result.push(difference & 1 == 1);
            borrow = i8::from(difference < 0);
        }
        (result, borrow == 1)
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

    fn expected_intrinsic(intrinsic: lapc_ir::Intrinsic, arguments: &[Vec<bool>]) -> Vec<bool> {
        let left = arguments[0].as_slice();
        let right = arguments.get(1).map(Vec::as_slice).unwrap_or(&[]);
        match intrinsic {
            lapc_ir::Intrinsic::Not => left.iter().map(|value| !value).collect(),
            lapc_ir::Intrinsic::And => left
                .iter()
                .zip(right)
                .map(|(left, right)| left & right)
                .collect(),
            lapc_ir::Intrinsic::Or => left
                .iter()
                .zip(right)
                .map(|(left, right)| left | right)
                .collect(),
            lapc_ir::Intrinsic::Xor => left
                .iter()
                .zip(right)
                .map(|(left, right)| left ^ right)
                .collect(),
            lapc_ir::Intrinsic::Add => add_values(left, right),
            lapc_ir::Intrinsic::Sub => sub_values(left, right),
            lapc_ir::Intrinsic::Mul => mul_values(left, right),
            lapc_ir::Intrinsic::Inc => add_values(left, &bits_of_value(1, left.len())),
            lapc_ir::Intrinsic::Dec => sub_values(left, &bits_of_value(1, left.len())),
            lapc_ir::Intrinsic::ShiftLeftOne => shift_left(left, 1),
            lapc_ir::Intrinsic::ShiftRightOne => shift_right(left, 1),
            lapc_ir::Intrinsic::Eq => vec![left == right],
            lapc_ir::Intrinsic::Lt => vec![unsigned_less_than(left, right)],
            lapc_ir::Intrinsic::IsZero => vec![left.iter().all(|bit| !bit)],
            lapc_ir::Intrinsic::Select => {
                let when_one = arguments[1].as_slice();
                let when_zero = arguments[2].as_slice();
                left.iter()
                    .zip(when_one)
                    .zip(when_zero)
                    .map(|((flag, one), zero)| if *flag { *one } else { *zero })
                    .collect()
            }
            lapc_ir::Intrinsic::DivMod => {
                let divisor = value_of(right);
                let (quotient, remainder) = if divisor == 0 {
                    (0, 0)
                } else {
                    (value_of(left) / divisor, value_of(left) % divisor)
                };
                let mut result = bits_of_value(quotient, left.len());
                result.extend(bits_of_value(remainder, left.len()));
                result
            }
            lapc_ir::Intrinsic::AddWithCarry => {
                let (sum, out) = add_with_carry_values(left, right, arguments[2][0]);
                let mut result = sum;
                result.push(out);
                result
            }
            lapc_ir::Intrinsic::SubWithBorrow => {
                let (difference, out) = sub_with_borrow_values(left, right, arguments[2][0]);
                let mut result = difference;
                result.push(out);
                result
            }
        }
    }

    fn assert_word(program: &Program, label: &str, expected: u64) {
        let Some(result) = run_word(program, label) else {
            return;
        };
        assert_eq!(result, expected, "for {label}");
    }

    fn assert_wide(program: &Program, label: &str, values: &[bool]) {
        let Some(result) = run_wide(program, label) else {
            return;
        };
        assert_eq!(result, packed_bytes(values), "for {label}");
    }

    #[test]
    fn runs_constants_at_every_width_boundary() {
        for width in [0usize, 1, 2, 7, 8, 63, 64, 65, 127, 128, 129, 192, 1000] {
            let values = pattern_bits(width, 1);
            let program = Program::new(vec![constant_function(
                "constant",
                flat_type(width),
                bits(&values),
            )]);
            if width <= 64 {
                assert_word(&program, "constant", bits_to_word(&values, width));
            } else {
                assert_wide(&program, "constant", &values);
            }
        }
    }

    #[test]
    fn runs_nand_at_every_width_boundary() {
        for width in [0usize, 1, 2, 7, 8, 63, 64, 65, 127, 128, 129, 192, 1000] {
            let left = pattern_bits(width, 1);
            let right = pattern_bits(width, 3);
            let expected: Vec<bool> = left
                .iter()
                .zip(&right)
                .map(|(left, right)| !(left & right))
                .collect();
            let value = Value::Nand(Box::new(bits(&left)), Box::new(bits(&right)));
            let program = Program::new(vec![constant_function("nand", flat_type(width), value)]);
            if width <= 64 {
                assert_word(&program, "nand", bits_to_word(&expected, width));
            } else {
                assert_wide(&program, "nand", &expected);
            }
        }
    }

    #[test]
    fn runs_every_intrinsic_at_every_width_boundary() {
        let intrinsics = [
            lapc_ir::Intrinsic::Not,
            lapc_ir::Intrinsic::And,
            lapc_ir::Intrinsic::Or,
            lapc_ir::Intrinsic::Xor,
            lapc_ir::Intrinsic::Add,
            lapc_ir::Intrinsic::Sub,
        ];
        for width in [0usize, 1, 2, 8, 63, 64, 65, 128, 192, 1000] {
            let mut elements = Vec::new();
            let mut expected = Vec::new();
            for intrinsic in intrinsics {
                let left = pattern_bits(width, 2);
                let right = pattern_bits(width, 4);
                let argument_bits = if intrinsic == lapc_ir::Intrinsic::Not {
                    vec![left.clone()]
                } else {
                    vec![left.clone(), right.clone()]
                };
                let arguments: Vec<Value> = argument_bits.iter().map(|bits_| bits(bits_)).collect();
                elements.push(Value::Intrinsic(intrinsic, arguments));
                expected.extend(expected_intrinsic(intrinsic, &argument_bits));
            }
            let program = Program::new(vec![constant_function(
                "intrinsic",
                Type::Collection(vec![flat_type(width); intrinsics.len()]),
                Value::Collection(elements),
            )]);
            if expected.len() <= 64 {
                assert_word(
                    &program,
                    "intrinsic",
                    bits_to_word(&expected, expected.len()),
                );
            } else {
                assert_wide(&program, "intrinsic", &expected);
            }
        }
    }

    #[test]
    fn runs_every_new_intrinsic() {
        let intrinsics = [
            lapc_ir::Intrinsic::Mul,
            lapc_ir::Intrinsic::Inc,
            lapc_ir::Intrinsic::Dec,
            lapc_ir::Intrinsic::ShiftLeftOne,
            lapc_ir::Intrinsic::ShiftRightOne,
            lapc_ir::Intrinsic::Eq,
            lapc_ir::Intrinsic::Lt,
            lapc_ir::Intrinsic::IsZero,
        ];
        for width in [1usize, 2, 4, 8, 16, 32, 63, 64] {
            let mut elements = Vec::new();
            let mut types = Vec::new();
            let mut expected = Vec::new();
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
                elements.push(Value::Intrinsic(intrinsic, arguments));
                types.push(result);
                expected.extend(expected_intrinsic(intrinsic, &argument_bits));
            }
            let program = Program::new(vec![constant_function(
                "intrinsic",
                Type::Collection(types),
                Value::Collection(elements),
            )]);
            if expected.len() <= 64 {
                assert_word(
                    &program,
                    "intrinsic",
                    bits_to_word(&expected, expected.len()),
                );
            } else {
                assert_wide(&program, "intrinsic", &expected);
            }
        }
    }

    #[test]
    fn runs_carry_intrinsics() {
        let intrinsics = [
            lapc_ir::Intrinsic::AddWithCarry,
            lapc_ir::Intrinsic::SubWithBorrow,
        ];
        for width in [1usize, 2, 4, 8, 16, 32, 63, 64] {
            let mut elements = Vec::new();
            let mut types = Vec::new();
            let mut expected = Vec::new();
            for intrinsic in intrinsics {
                for carry in [false, true] {
                    let left = pattern_bits(width, 2);
                    let right = pattern_bits(width, 4);
                    let argument_bits = vec![left.clone(), right.clone(), vec![carry]];
                    let arguments: Vec<Value> =
                        argument_bits.iter().map(|bits_| bits(bits_)).collect();
                    elements.push(Value::Intrinsic(intrinsic, arguments));
                    types.push(Type::Collection(vec![flat_type(width), Type::Bit]));
                    expected.extend(expected_intrinsic(intrinsic, &argument_bits));
                }
            }
            let program = Program::new(vec![constant_function(
                "intrinsic",
                Type::Collection(types),
                Value::Collection(elements),
            )]);
            if expected.len() <= 64 {
                assert_word(
                    &program,
                    "intrinsic",
                    bits_to_word(&expected, expected.len()),
                );
            } else {
                assert_wide(&program, "intrinsic", &expected);
            }
        }
    }

    #[test]
    fn multiplies_by_a_constant() {
        let constants = [0u128, 1, 2, 3, 5, 7, 9, 11, 25, 100, 255];
        for width in [0usize, 1, 2, 4, 8, 16, 32, 63, 64] {
            let left = pattern_bits(width, 2);
            let mut elements = Vec::new();
            let mut expected = Vec::new();
            for constant in constants {
                let right = bits_of_value(constant & mask_wide(width), width);
                elements.push(Value::Intrinsic(
                    lapc_ir::Intrinsic::Mul,
                    vec![bits(&left), bits(&right)],
                ));
                expected.extend(mul_values(&left, &right));
            }
            let program = Program::new(vec![constant_function(
                "multiply",
                Type::Collection(vec![flat_type(width); constants.len()]),
                Value::Collection(elements),
            )]);
            if expected.len() <= 64 {
                assert_word(
                    &program,
                    "multiply",
                    bits_to_word(&expected, expected.len()),
                );
            } else {
                assert_wide(&program, "multiply", &expected);
            }
        }
    }

    #[test]
    fn runs_select() {
        for (flag, when_one, when_zero) in [
            (false, false, false),
            (false, true, false),
            (true, false, true),
            (true, true, true),
        ] {
            let value = Value::Intrinsic(
                lapc_ir::Intrinsic::Select,
                vec![bit(flag), bit(when_one), bit(when_zero)],
            );
            let program = Program::new(vec![constant_function("select", Type::Bit, value)]);
            let expected = if flag { when_one } else { when_zero };
            assert_word(&program, "select", u64::from(expected));
        }
    }

    #[test]
    fn masks_a_word_at_width_64() {
        let invert = Program::new(vec![constant_function(
            "operation",
            flat_type(64),
            Value::Intrinsic(lapc_ir::Intrinsic::Not, vec![bits(&[false; 64])]),
        )]);
        assert_word(&invert, "operation", u64::MAX);
        let high_bit = bits_of_value(1u128 << 63, 64);
        let wrap = Program::new(vec![constant_function(
            "operation",
            flat_type(64),
            Value::Intrinsic(
                lapc_ir::Intrinsic::Add,
                vec![bits(&high_bit), bits(&high_bit)],
            ),
        )]);
        assert_word(&wrap, "operation", 0);
        let borrow = Program::new(vec![constant_function(
            "operation",
            flat_type(64),
            Value::Intrinsic(
                lapc_ir::Intrinsic::Sub,
                vec![bits(&[false; 64]), bits(&bits_of_value(1, 64))],
            ),
        )]);
        assert_word(&borrow, "operation", u64::MAX);
    }

    #[test]
    fn runs_a_constant() {
        let program = Program::new(vec![constant_function("main", Type::Bit, bit(true))]);
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, 1);
    }

    #[test]
    fn runs_a_nand() {
        let value = Value::Nand(Box::new(bit(true)), Box::new(bit(false)));
        let program = Program::new(vec![constant_function("main", Type::Bit, value)]);
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, 1);
    }

    #[test]
    fn runs_a_checked_program() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nbit.not = (a: BIT) BIT { NAND(a, a) }\nbit.toggle = (target: *BIT) [] { target = bit.not(target) }\nmain = () BIT { state: BIT = BIT.ZERO\n bit.toggle(*state)\n state }\n";
        let program = compile_source(source);
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, 1);
    }

    #[test]
    fn runs_a_reference_to_an_element() {
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
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, 1);
    }

    #[test]
    fn runs_a_wide_constant() {
        let mut values = vec![false; 65];
        values[64] = true;
        let program = Program::new(vec![constant_function(
            "wide",
            flat_type(65),
            bits(&values),
        )]);
        let Some(result) = run_wide(&program, "wide") else {
            return;
        };
        assert_eq!(result, packed_bytes(&values));
    }

    #[test]
    fn runs_a_wide_parameter() {
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
        let mut input = vec![0u64; 2];
        input[1] = 1;
        let Some(result) = run_wide_call(&program, "identity", &[Argument::Wide(input)]) else {
            return;
        };
        assert_eq!(result, vec![0, 0, 0, 0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn runs_a_wide_add() {
        let mut left = vec![false; 65];
        left[64] = true;
        let mut right = vec![false; 65];
        right[0] = true;
        let value = Value::Intrinsic(lapc_ir::Intrinsic::Add, vec![bits(&left), bits(&right)]);
        let program = Program::new(vec![constant_function("add", flat_type(65), value)]);
        let Some(result) = run_wide(&program, "add") else {
            return;
        };
        assert_eq!(result, vec![1, 0, 0, 0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn runs_a_wide_branch() {
        let wide = Type::Collection(vec![Type::Bit; 65]);
        let mut values = vec![false; 65];
        values[64] = true;
        let main = function(
            "main",
            vec![],
            wide.clone(),
            vec![],
            block(
                vec![],
                Value::Branch(
                    Box::new(bit(true)),
                    block(vec![], bits(&values)),
                    block(vec![], bits(&[false; 65])),
                ),
            ),
        );
        let program = Program::new(vec![main]);
        let Some(result) = run_wide(&program, "main") else {
            return;
        };
        assert_eq!(result, packed_bytes(&values));
    }

    #[test]
    fn runs_a_wide_call() {
        let wide = Type::Collection(vec![Type::Bit; 65]);
        let mut values = vec![false; 65];
        values[64] = true;
        let callee = function(
            "callee",
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
        let main = function(
            "main",
            vec![],
            wide.clone(),
            vec![wide.clone()],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: bits(&values),
                }],
                Value::Call(
                    Label::new("callee"),
                    vec![Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: 0,
                    }))],
                ),
            ),
        );
        let program = Program::new(vec![callee, main]);
        let Some(result) = run_wide(&program, "main") else {
            return;
        };
        assert_eq!(result, packed_bytes(&values));
    }

    #[test]
    fn runs_a_reference_to_a_word_at_every_offset() {
        let values = pattern_bits(64, 1);
        let elements: Vec<Value> = (0..64)
            .map(|offset| {
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: offset,
                }))
            })
            .collect();
        let main = function(
            "main",
            vec![],
            flat_type(64),
            vec![flat_type(64)],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: bits(&values),
                }],
                Value::Collection(elements),
            ),
        );
        let program = Program::new(vec![main]);
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, bits_to_word(&values, 64));
    }

    #[test]
    fn runs_a_reference_to_a_wide_element_at_every_offset() {
        let values = pattern_bits(65, 3);
        let elements: Vec<Value> = (0..65)
            .map(|offset| {
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: offset,
                }))
            })
            .collect();
        let main = function(
            "main",
            vec![],
            flat_type(65),
            vec![flat_type(65)],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: bits(&values),
                }],
                Value::Collection(elements),
            ),
        );
        let program = Program::new(vec![main]);
        let Some(result) = run_wide(&program, "main") else {
            return;
        };
        assert_eq!(result, packed_bytes(&values));
    }

    #[test]
    fn runs_a_reference_parameter_at_a_bit_offset() {
        let read = function(
            "read",
            vec![Parameter::new(Slot::new(0))],
            Type::Bit,
            vec![Type::Reference(Box::new(Type::Bit))],
            block(
                vec![],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        let values = pattern_bits(64, 5);
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![flat_type(64)],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: bits(&values),
                }],
                Value::Call(
                    Label::new("read"),
                    vec![Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: 37,
                    }],
                ),
            ),
        );
        let program = Program::new(vec![read, main]);
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, u64::from(values[37]));
    }

    #[test]
    fn runs_a_store_through_a_reference_parameter_at_a_bit_offset() {
        let set = function(
            "set",
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
            vec![flat_type(64)],
            block(
                vec![
                    Statement::Bind {
                        slot: Slot::new(0),
                        value: bits(&[false; 64]),
                    },
                    Statement::Evaluate {
                        value: Value::Call(
                            Label::new("set"),
                            vec![Value::Reference {
                                slot: Slot::new(0),
                                bit_offset: 37,
                            }],
                        ),
                    },
                ],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 37,
                })),
            ),
        );
        let program = Program::new(vec![set, main]);
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, 1);
    }

    #[test]
    fn runs_a_wide_element_of_a_wide_collection() {
        let wide = Type::Collection(vec![Type::Collection(vec![Type::Bit; 64]), Type::Bit]);
        let mut low = vec![false; 64];
        low[3] = true;
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![wide.clone()],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: Value::Collection(vec![bits(&low), bit(true)]),
                }],
                Value::Element {
                    collection: Box::new(Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(0),
                        bit_offset: 0,
                    }))),
                    index: 1,
                },
            ),
        );
        let program = Program::new(vec![main]);
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, 1);
    }

    #[test]
    fn runs_a_wide_element_at_a_bit_offset() {
        let wide = Type::Collection(vec![Type::Bit, Type::Collection(vec![Type::Bit; 64])]);
        let mut high = vec![false; 64];
        high[3] = true;
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![wide.clone()],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: Value::Collection(vec![bit(true), bits(&high)]),
                }],
                Value::Element {
                    collection: Box::new(Value::Element {
                        collection: Box::new(Value::Load(Box::new(Value::Reference {
                            slot: Slot::new(0),
                            bit_offset: 0,
                        }))),
                        index: 1,
                    }),
                    index: 3,
                },
            ),
        );
        let program = Program::new(vec![main]);
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, 1);
    }

    #[test]
    fn runs_a_wide_store_at_a_bit_offset() {
        let wide = Type::Collection(vec![Type::Bit, Type::Collection(vec![Type::Bit; 64])]);
        let mut high = vec![false; 64];
        high[3] = true;
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![
                wide.clone(),
                Type::Reference(Box::new(Type::Collection(vec![Type::Bit; 64]))),
            ],
            block(
                vec![
                    Statement::Bind {
                        slot: Slot::new(0),
                        value: Value::Collection(vec![bit(false), bits(&[false; 64])]),
                    },
                    Statement::Bind {
                        slot: Slot::new(1),
                        value: Value::Reference {
                            slot: Slot::new(0),
                            bit_offset: 1,
                        },
                    },
                    Statement::Store {
                        reference: Value::Reference {
                            slot: Slot::new(1),
                            bit_offset: 0,
                        },
                        value: bits(&high),
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
                    index: 3,
                },
            ),
        );
        let program = Program::new(vec![main]);
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, 1);
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
    fn runs_a_nand_of_zeros() {
        let value = Value::Nand(Box::new(bit(false)), Box::new(bit(false)));
        let program = Program::new(vec![constant_function("main", Type::Bit, value)]);
        assert_word(&program, "main", 1);
    }

    #[test]
    fn runs_a_bit_branch() {
        let value = Value::Branch(
            Box::new(bit(true)),
            block(vec![], bit(false)),
            block(vec![], bit(true)),
        );
        let program = Program::new(vec![constant_function("main", Type::Bit, value)]);
        assert_word(&program, "main", 0);
    }

    #[test]
    fn runs_a_wide_element() {
        let mut values = vec![false; 65];
        values[64] = true;
        let value = Value::Element {
            collection: Box::new(bits(&values)),
            index: 64,
        };
        let program = Program::new(vec![constant_function("main", Type::Bit, value)]);
        assert_word(&program, "main", 1);
    }

    #[test]
    fn runs_a_wide_copy() {
        let wide = flat_type(65);
        let mut values = vec![false; 65];
        values[64] = true;
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![wide.clone(), wide.clone()],
            block(
                vec![
                    Statement::Bind {
                        slot: Slot::new(0),
                        value: bits(&values),
                    },
                    Statement::Bind {
                        slot: Slot::new(1),
                        value: Value::Load(Box::new(Value::Reference {
                            slot: Slot::new(0),
                            bit_offset: 0,
                        })),
                    },
                ],
                Value::Element {
                    collection: Box::new(Value::Load(Box::new(Value::Reference {
                        slot: Slot::new(1),
                        bit_offset: 0,
                    }))),
                    index: 64,
                },
            ),
        );
        let program = Program::new(vec![main]);
        assert_word(&program, "main", 1);
    }

    #[test]
    fn keeps_every_bit_at_width_64() {
        let mut values = pattern_bits(64, 2);
        values[63] = true;
        let main = function(
            "main",
            vec![],
            flat_type(64),
            vec![flat_type(64)],
            block(
                vec![Statement::Bind {
                    slot: Slot::new(0),
                    value: bits(&values),
                }],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        let program = Program::new(vec![main]);
        assert_word(&program, "main", bits_to_word(&values, 64));
    }

    #[test]
    fn runs_a_nand_of_wide_elements() {
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
        assert_word(&program, "nand", 0);
    }

    #[test]
    fn runs_a_self_tail_call_as_a_loop() {
        let program = compile_source(&format!(
            "{COUNTING_PRELUDE}\
         count.down = (count: U2) BIT {{ BRANCH (u2.is.zero(count)) {{ BIT.ONE }} {{ count.down(u2.dec(count)) }} }}\n\
         main = () BIT {{ count.down(U2.THREE) }}\n"
        ));
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, 1);
    }

    #[test]
    fn runs_a_tail_call_with_a_local_reference_as_a_call() {
        let program = compile_source(&format!(
            "{COUNTING_PRELUDE}\
         count.down = (count: U2, target: *U2) BIT {{ BRANCH (u2.is.zero(count)) {{ [low, high] = target\n low }} {{ count.down(u2.dec(count), *count) }} }}\n\
         main = () BIT {{ count: U2 = U2.THREE\n count.down(count, *count) }}\n"
        ));
        let Some(result) = run_word(&program, "main") else {
            return;
        };
        assert_eq!(result, 1);
    }

    #[test]
    fn runs_nand_of_structured_collections() {
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
        assert_word(&program, "nand", 3);
    }

    #[test]
    fn runs_an_intrinsic_of_structured_collections() {
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
        let value = Value::Intrinsic(lapc_ir::Intrinsic::Xor, vec![left, right]);
        let program = Program::new(vec![constant_function("xor", type_, value)]);
        assert_word(&program, "xor", 3);
    }

    #[test]
    fn runs_a_collection_of_structured_elements() {
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
        assert_word(&program, "collection", 13);
    }

    #[test]
    fn runs_every_element_of_a_structured_collection() {
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
        let mut values = Vec::new();
        let mut expected = Vec::new();
        for (index, element) in elements.iter().enumerate() {
            values.push(Value::Element {
                collection: Box::new(collection.clone()),
                index,
            });
            expected.push(element.clone());
        }
        let program = Program::new(vec![constant_function(
            "element",
            Type::Collection(expected),
            Value::Collection(values),
        )]);
        assert_word(&program, "element", 1 | (1 << 2) | (6 << 3));
    }

    #[test]
    fn runs_a_nested_element_of_mixed_widths() {
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
        assert_word(&program, "element", 0);
    }

    #[test]
    fn runs_a_source_element_after_a_narrow_element() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nmain = () BIT { bundle: [BIT, [BIT, BIT]] = [BIT.ONE, [BIT.ZERO, BIT.ONE]]\n [first, second] = bundle\n [low, high] = second\n high }\n";
        assert_word(&compile_source(source), "main", 1);
    }

    #[test]
    fn runs_a_source_element_after_a_wide_element() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nmain = () BIT { bundle: [BIT, [BIT, BIT], BIT] = [BIT.ONE, [BIT.ONE, BIT.ONE], BIT.ZERO]\n [first, second, third] = bundle\n third }\n";
        assert_word(&compile_source(source), "main", 0);
    }

    #[test]
    fn runs_an_element_of_mixed_widths() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nmain = () BIT { both: [[BIT, BIT], BIT] = [[BIT.ZERO, BIT.ONE], BIT.ZERO]\n [pair, flag] = both\n flag }\n";
        assert_word(&compile_source(source), "main", 0);
    }

    #[test]
    fn runs_an_ignored_element() {
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
        assert_word(&program, "main", 1);
    }

    #[test]
    fn runs_zero_width_values() {
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
        assert_word(&program, "caller", 1);
        let program = Program::new(vec![producer]);
        assert_word(&program, "empty", 0);
    }

    #[test]
    fn runs_a_zero_width_argument() {
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
        assert_word(&program, "caller", 1);
    }

    #[test]
    fn runs_a_wide_call_with_two_wide_arguments() {
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
        let expected: Vec<bool> = left
            .iter()
            .zip(&right)
            .map(|(left, right)| !(left & right))
            .collect();
        let Some(result) = run_wide_call(
            &program,
            "wide_nand",
            &[
                Argument::Wide(vec![value_of(&left) as u64, (value_of(&left) >> 64) as u64]),
                Argument::Wide(vec![
                    value_of(&right) as u64,
                    (value_of(&right) >> 64) as u64,
                ]),
            ],
        ) else {
            return;
        };
        assert_eq!(result, packed_bytes(&expected));
    }

    #[test]
    fn runs_a_wide_call_result_as_a_wide_argument() {
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
                    vec![Value::Intrinsic(
                        lapc_ir::Intrinsic::Not,
                        vec![bits(&values)],
                    )],
                ),
            ),
        );
        let program = Program::new(vec![identity, wrapper]);
        let expected = bits_of_value(!value_of(&values) & mask_wide(65), 65);
        assert_wide(&program, "wrapper", &expected);
    }

    #[test]
    fn runs_a_wide_call_from_a_wide_parameter() {
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
        let Some(result) = run_wide_call(
            &program,
            "forward",
            &[Argument::Wide(vec![
                value_of(&values) as u64,
                (value_of(&values) >> 64) as u64,
            ])],
        ) else {
            return;
        };
        assert_eq!(result, packed_bytes(&values));
    }

    #[test]
    fn runs_an_empty_branch() {
        let empty = Type::Collection(vec![]);
        let value = Value::Branch(
            Box::new(bit(true)),
            empty_block(vec![]),
            empty_block(vec![Statement::Evaluate { value: bit(false) }]),
        );
        let program = Program::new(vec![constant_function("branch", empty, value)]);
        assert_word(&program, "branch", 0);
    }

    #[test]
    fn runs_a_branch_with_a_wide_result() {
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
            assert_wide(&program, "branch", expected);
        }
    }

    #[test]
    fn runs_a_nested_branch_with_a_wide_result() {
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
        assert_wide(&program, "branch", &pattern_bits(65, 3));
    }

    #[test]
    fn runs_a_branch_with_a_wide_binding() {
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
        assert_wide(&program, "branch", &then_values);
    }

    #[test]
    fn runs_a_division_at_every_width_boundary() {
        let cases = [
            (7u128, 2u128),
            (1000, 10),
            (0, 5),
            (5, 0),
            (0, 0),
            (255, 1),
            (123456789, 1000000),
        ];
        for width in [1usize, 4, 8, 9, 16, 32, 63, 64] {
            let mut elements = Vec::new();
            let mut expected = Vec::new();
            for (value, divisor) in cases {
                let value = value & mask_wide(width);
                let divisor = divisor & mask_wide(width);
                elements.push(Value::Intrinsic(
                    lapc_ir::Intrinsic::DivMod,
                    vec![
                        bits(&bits_of_value(value, width)),
                        bits(&bits_of_value(divisor, width)),
                    ],
                ));
                let (quotient, remainder) = if divisor == 0 {
                    (0, 0)
                } else {
                    (value / divisor, value % divisor)
                };
                expected.extend(bits_of_value(quotient, width));
                expected.extend(bits_of_value(remainder, width));
            }
            let pair = Type::Collection(vec![flat_type(width), flat_type(width)]);
            let program = Program::new(vec![constant_function(
                "divide",
                Type::Collection(vec![pair; cases.len()]),
                Value::Collection(elements),
            )]);
            if expected.len() <= 64 {
                assert_word(&program, "divide", bits_to_word(&expected, expected.len()));
            } else {
                assert_wide(&program, "divide", &expected);
            }
        }
    }

    #[test]
    fn runs_a_division_by_a_runtime_divisor() {
        for width in [4usize, 8, 16, 32, 64] {
            let operand_type = flat_type(width);
            let divide = function(
                "divide",
                vec![Parameter::new(Slot::new(0)), Parameter::new(Slot::new(1))],
                Type::Collection(vec![operand_type.clone(), operand_type.clone()]),
                vec![operand_type.clone(), operand_type.clone()],
                block(
                    vec![],
                    Value::Intrinsic(
                        lapc_ir::Intrinsic::DivMod,
                        vec![
                            Value::Load(Box::new(Value::Reference {
                                slot: Slot::new(0),
                                bit_offset: 0,
                            })),
                            Value::Load(Box::new(Value::Reference {
                                slot: Slot::new(1),
                                bit_offset: 0,
                            })),
                        ],
                    ),
                ),
            );
            let program = Program::new(vec![divide]);
            for (value, divisor) in [
                (0u64, 0u64),
                (1, 0),
                (7, 2),
                (255, 10),
                (12345, 67),
                (1 << 63, 3),
            ] {
                let value = u128::from(value) & mask_wide(width);
                let divisor = u128::from(divisor) & mask_wide(width);
                let Some(bytes) = run_wide_call(
                    &program,
                    "divide",
                    &[Argument::Word(value as u64), Argument::Word(divisor as u64)],
                ) else {
                    return;
                };
                let all: Vec<bool> = (0..2 * width)
                    .map(|index| bytes[index / 8] >> (index % 8) & 1 == 1)
                    .collect();
                let (quotient, remainder) = (value_of(&all[..width]), value_of(&all[width..]));
                let expected = if divisor == 0 {
                    (0, 0)
                } else {
                    (value / divisor, value % divisor)
                };
                assert_eq!(
                    (quotient, remainder),
                    expected,
                    "for width {width}, {value} divided by {divisor}"
                );
            }
        }
    }

    #[test]
    fn runs_an_erased_program() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nU8 = [BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]\nintrinsic.u8.add.with.carry = (left: U8, right: U8, carry: BIT) [U8, BIT] { [[BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO], BIT.ZERO] }\nmain = () BIT { [sum, carry] = intrinsic.u8.add.with.carry([BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ZERO], [BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO], BIT.ZERO)\n [b0, b1, b2, b3, b4, b5, b6, b7] = sum\n b7 }\n";
        assert_word(&compile_source(source), "main", 1);
    }

    #[test]
    fn runs_every_erased_intrinsic() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nU8 = [BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]\nintrinsic.bit.not = (a: BIT) BIT { NAND(a, a) }\nintrinsic.bit.exclusive.or = (a: BIT, b: BIT) BIT { BIT.ZERO }\nintrinsic.bit.or = (a: BIT, b: BIT) BIT { BIT.ZERO }\nintrinsic.bit.and = (a: BIT, b: BIT) BIT { BIT.ZERO }\nintrinsic.u8.sub.with.borrow = (left: U8, right: U8, borrow: BIT) [U8, BIT] { [[BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO], BIT.ZERO] }\nintrinsic.u8.add.with.carry = (left: U8, right: U8, carry: BIT) [U8, BIT] { [[BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO], BIT.ZERO] }\nmain = () BIT { plain: BIT = intrinsic.bit.and(intrinsic.bit.not(BIT.ZERO), intrinsic.bit.or(BIT.ZERO, intrinsic.bit.exclusive.or(BIT.ONE, BIT.ZERO)))\n [difference, borrow] = intrinsic.u8.sub.with.borrow([BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO], [BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO], BIT.ZERO)\n [b0, b1, b2, b3, b4, b5, b6, b7] = difference\n NAND(plain, b7) }\n";
        assert_word(&compile_source(source), "main", 0);
    }

    #[test]
    fn runs_an_erased_intrinsic_label() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nintrinsic.bit.not = (value: BIT) BIT { NAND(value, value) }\nmain = () BIT { intrinsic.bit.not(BIT.ZERO) }\n";
        assert_word(&compile_source(source), "main", 1);
    }

    #[test]
    fn runs_an_erased_add_with_carry() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nU8 = [BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]\nU8.MAX = [BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE, BIT.ONE]\nU8.ONE = [BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO]\nintrinsic.u8.add.with.carry = (left: U8, right: U8, carry: BIT) [U8, BIT] { [left, BIT.ZERO] }\nmain = () BIT { [sum, carry] = intrinsic.u8.add.with.carry(U8.MAX, U8.ONE, BIT.ZERO)\n carry }\n";
        assert_word(&compile_source(source), "main", 1);
    }

    #[test]
    fn runs_an_erased_sub_with_borrow() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\nU8 = [BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]\nU8.ZERO = [BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO]\nU8.ONE = [BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO]\nintrinsic.u8.sub.with.borrow = (left: U8, right: U8, borrow: BIT) [U8, BIT] { [left, BIT.ZERO] }\nmain = () BIT { [difference, borrow] = intrinsic.u8.sub.with.borrow(U8.ZERO, U8.ONE, BIT.ZERO)\n borrow }\n";
        assert_word(&compile_source(source), "main", 1);
    }

    #[test]
    fn runs_an_extern_with_a_word_result() {
        let result = lapc_ir::extern_result_type(
            lapc_extern::lookup(0x0203).expect("the operation is known"),
        );
        let program = Program::new(vec![constant_function(
            "call",
            result,
            Value::Extern(
                lapc_extern::Operation::new(0x0203),
                vec![bits(&bits_of_value(1, 64))],
            ),
        )]);
        assert_word(&program, "call", 1);
    }

    #[test]
    fn runs_an_extern_with_a_wide_result() {
        let result = lapc_ir::extern_result_type(
            lapc_extern::lookup(0x0100).expect("the operation is known"),
        );
        let program = Program::new(vec![constant_function(
            "call",
            result,
            Value::Extern(
                lapc_extern::Operation::new(0x0100),
                vec![bits(&bits_of_value(8, 64))],
            ),
        )]);
        assert_wide(&program, "call", &bits_of_value(1, 65));
    }

    #[test]
    fn runs_an_extern_chain() {
        let acquired = lapc_ir::extern_result_type(
            lapc_extern::lookup(0x0100).expect("the operation is known"),
        );
        let written = lapc_ir::extern_result_type(
            lapc_extern::lookup(0x0103).expect("the operation is known"),
        );
        let read = lapc_ir::extern_result_type(
            lapc_extern::lookup(0x0102).expect("the operation is known"),
        );
        let handle = Value::Element {
            collection: Box::new(Value::Load(Box::new(Value::Reference {
                slot: Slot::new(0),
                bit_offset: 0,
            }))),
            index: 1,
        };
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![acquired.clone(), written.clone(), read.clone()],
            block(
                vec![
                    Statement::Bind {
                        slot: Slot::new(0),
                        value: Value::Extern(
                            lapc_extern::Operation::new(0x0100),
                            vec![bits(&bits_of_value(8, 64))],
                        ),
                    },
                    Statement::Bind {
                        slot: Slot::new(1),
                        value: Value::Extern(
                            lapc_extern::Operation::new(0x0103),
                            vec![
                                handle.clone(),
                                bits(&bits_of_value(0, 64)),
                                bits(&bits_of_value(0xab, 8)),
                            ],
                        ),
                    },
                    Statement::Bind {
                        slot: Slot::new(2),
                        value: Value::Extern(
                            lapc_extern::Operation::new(0x0102),
                            vec![handle, bits(&bits_of_value(0, 64))],
                        ),
                    },
                ],
                Value::Element {
                    collection: Box::new(Value::Element {
                        collection: Box::new(Value::Load(Box::new(Value::Reference {
                            slot: Slot::new(2),
                            bit_offset: 0,
                        }))),
                        index: 1,
                    }),
                    index: 0,
                },
            ),
        );
        let program = Program::new(vec![main]);
        assert_word(&program, "main", 1);
    }

    #[test]
    fn runs_a_reference_argument() {
        let word = flat_type(64);
        let read = function(
            "read",
            vec![Parameter::new(Slot::new(0))],
            word.clone(),
            vec![Type::Reference(Box::new(word.clone()))],
            block(
                vec![],
                Value::Load(Box::new(Value::Reference {
                    slot: Slot::new(0),
                    bit_offset: 0,
                })),
            ),
        );
        let program = Program::new(vec![read]);
        let values = pattern_bits(64, 9);
        let stored = bits_to_word(&values, 64);
        for bit in [0u64, 37] {
            let Some(result) = run_word_call(
                &program,
                "read",
                &[Argument::Reference {
                    storage: vec![stored, 0],
                    bit,
                }],
            ) else {
                return;
            };
            let expected = if bit == 0 { stored } else { stored >> bit };
            assert_eq!(result, expected, "at bit {bit}");
        }
    }

    #[test]
    fn runs_an_extern_payload_bit() {
        let result = lapc_ir::extern_result_type(
            lapc_extern::lookup(0x0100).expect("the operation is known"),
        );
        let acquire = || {
            Value::Extern(
                lapc_extern::Operation::new(0x0100),
                vec![bits(&bits_of_value(0, 64))],
            )
        };
        let main = function(
            "main",
            vec![],
            Type::Bit,
            vec![result.clone(), result.clone()],
            block(
                vec![
                    Statement::Bind {
                        slot: Slot::new(0),
                        value: acquire(),
                    },
                    Statement::Bind {
                        slot: Slot::new(1),
                        value: acquire(),
                    },
                ],
                Value::Element {
                    collection: Box::new(Value::Element {
                        collection: Box::new(Value::Load(Box::new(Value::Reference {
                            slot: Slot::new(1),
                            bit_offset: 0,
                        }))),
                        index: 1,
                    }),
                    index: 0,
                },
            ),
        );
        let program = Program::new(vec![main]);
        assert_word(&program, "main", 1);
    }

    #[test]
    fn runs_a_destructured_single_bit_collection_constant() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\n\
            BIT.ZERO = NAND(BIT.ONE, BIT.ONE)\n\
            ONE = [BIT]\n\
            ONE.ONE = [BIT.ONE]\n\
            main = () BIT { [x] = ONE.ONE\n x }\n";
        let program = compile_source(source);
        assert_word(&program, "main", 1);
    }

    #[test]
    fn runs_a_destructured_nested_collection_constant() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\n\
            BIT.ZERO = NAND(BIT.ONE, BIT.ONE)\n\
            U2 = [BIT, BIT]\n\
            U2.ONE = [BIT.ONE, BIT.ZERO]\n\
            WRAP = [U2]\n\
            WRAP.ONE = [U2.ONE]\n\
            main = () BIT { [x] = WRAP.ONE\n [a, b] = x\n a }\n";
        let program = compile_source(source);
        assert_word(&program, "main", 1);
    }

    #[test]
    fn runs_a_destructured_branch_of_a_constant() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\n\
            BIT.ZERO = NAND(BIT.ONE, BIT.ONE)\n\
            U2 = [BIT, BIT]\n\
            U2.ONE = [BIT.ONE, BIT.ZERO]\n\
            U2.ZERO = [BIT.ZERO, BIT.ZERO]\n\
            WRAP = [U2]\n\
            WRAP.ONE = [U2.ONE]\n\
            WRAP.ZERO = [U2.ZERO]\n\
            main = () BIT { [x] = BRANCH (BIT.ONE) { WRAP.ONE } { WRAP.ZERO }\n [a, b] = x\n a }\n";
        let program = compile_source(source);
        assert_word(&program, "main", 1);
    }

    #[test]
    fn runs_a_collection_of_a_constant_element() {
        let source = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\n\
            BIT.ZERO = NAND(BIT.ONE, BIT.ONE)\n\
            U8 = [BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]\n\
            U32 = [U8, U8, U8, U8]\n\
            U64 = [U32, U32]\n\
            U8.ZERO = [BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO]\n\
            U8.ONE = [BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO]\n\
            U32.ZERO = [U8.ZERO, U8.ZERO, U8.ZERO, U8.ZERO]\n\
            U32.ONE = [U8.ONE, U8.ZERO, U8.ZERO, U8.ZERO]\n\
            pack = (low: U32) U64 { [low, U32.ONE] }\n\
            main = () BIT { pack(U32.ZERO)\n BIT.ZERO }\n";
        let program = compile_source(source);
        let Some(result) = run_word_call(&program, "pack", &[Argument::Word(0x1234)]) else {
            return;
        };
        assert_eq!(result, 0x1234 | (1 << 32));
    }
}
