use std::collections::HashMap;
use thiserror::Error;

use crate::parser::{
    Block, Expression, FunctionDefinition, Identifier, Program, Statement, Target, Type,
};

#[derive(Debug, PartialEq, Eq, Error)]
pub enum SemanticError {
    #[error("Undefined label: {0}")]
    UndefinedLabel(Identifier),

    #[error("Duplicate definition: {0}")]
    DuplicateLabel(Identifier),

    #[error("Label `{0}` is not a value")]
    NotAValue(Identifier),

    #[error("Label `{0}` is not a function")]
    NotAFunction(Identifier),

    #[error("Label `{0}` is not a reference")]
    NotARef(Identifier),

    #[error("Label `{0}` is not a valid type (no value bound with that name)")]
    UndefinedType(Identifier),

    #[error("Expected a value, got a function")]
    ExpectedValueGotFunction,

    #[error("Function targets must be a plain identifier")]
    InvalidFunctionTarget,

    #[error("Type mismatch: expected {expected}, found {found}")]
    TypeMismatch { expected: Type, found: Type },

    #[error("Cannot unbind non-collection type: {0}")]
    UnbindNonCollection(Type),

    #[error("Unbind arity mismatch: expected {expected} target(s), found {found}")]
    UnbindArityMismatch { expected: usize, found: usize },

    #[error("NAND requires BIT operands, got {0}")]
    NandRequiresBit(Type),

    #[error("BRANCH control must be BIT, got {0}")]
    BranchControlNotBit(Type),

    #[error("BRANCH branches must have the same type: {0} vs {1}")]
    BranchTypeMismatch(Type, Type),

    #[error("Function `{label}` expects {expected} argument(s), got {found}")]
    ArgumentCountMismatch {
        label: Identifier,
        expected: usize,
        found: usize,
    },

    #[error("Function `{label}` argument {index}: expected {expected}, found {found}")]
    ArgumentTypeMismatch {
        label: Identifier,
        index: usize,
        expected: Type,
        found: Type,
    },

    #[error("Functions cannot return reference types, got {0}")]
    FunctionReturnsReference(Type),

    #[error("`main` must be a function with no parameters returning BIT")]
    InvalidMainSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Binding {
    Value(Type),
    Function {
        parameters: Vec<Type>,
        return_type: Type,
    },
}

#[derive(Debug)]
struct ScopeStack {
    frames: Vec<HashMap<Identifier, Binding>>,
}

impl ScopeStack {
    fn new() -> Self {
        Self {
            frames: vec![HashMap::new()],
        }
    }

    fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.frames.pop();
    }

    fn define(&mut self, name: Identifier, binding: Binding) -> Result<(), SemanticError> {
        let frame = self.frames.last_mut().expect("at least one frame");
        if frame.contains_key(&name) {
            return Err(SemanticError::DuplicateLabel(name));
        }
        frame.insert(name, binding);
        Ok(())
    }

    fn lookup(&self, name: &str) -> Option<&Binding> {
        for frame in self.frames.iter().rev() {
            if let Some(binding) = frame.get(name) {
                return Some(binding);
            }
        }
        None
    }
}

pub fn analyze(program: &Program) -> Result<(), SemanticError> {
    let mut scope = ScopeStack::new();
    let mut deferred_bodies: Vec<&FunctionDefinition> = Vec::new();

    for stmt in &program.statements {
        match stmt {
            Statement::Binding(binding) => {
                if let Expression::FunctionDefinition(func) = &binding.value {
                    hoist_function(&binding.target, func, &mut scope)?;
                    deferred_bodies.push(func);
                } else {
                    let value = check_expression(&binding.value, &mut scope)?;
                    register_target(&binding.target, value, &mut scope)?;
                }
            }
            Statement::Expression(expr) => {
                check_expression(expr, &mut scope)?;
            }
        }
    }

    for func in &deferred_bodies {
        check_function_definition(func, &mut scope)?;
    }

    validate_main(&scope)
}

fn hoist_function(
    target: &Target,
    func: &FunctionDefinition,
    scope: &mut ScopeStack,
) -> Result<(), SemanticError> {
    let name = target_name(target)?;
    let mut param_tys = Vec::with_capacity(func.parameters.len());
    for p in &func.parameters {
        param_tys.push(resolve_type(&p.r#type, scope)?);
    }
    let return_ty = resolve_type(&func.return_type, scope)?;
    if matches!(return_ty, Type::Reference(_)) {
        return Err(SemanticError::FunctionReturnsReference(return_ty));
    }
    scope.define(
        name.to_string(),
        Binding::Function {
            parameters: param_tys,
            return_type: return_ty,
        },
    )
}

fn validate_main(scope: &ScopeStack) -> Result<(), SemanticError> {
    match scope.lookup("main") {
        Some(Binding::Function {
            parameters,
            return_type,
        }) => {
            if parameters.is_empty() && *return_type == Type::Bit {
                Ok(())
            } else {
                Err(SemanticError::InvalidMainSignature)
            }
        }
        _ => Err(SemanticError::InvalidMainSignature),
    }
}

fn target_name(target: &Target) -> Result<&str, SemanticError> {
    match target {
        Target::Identifier(ident) => Ok(&ident.name),
        _ => Err(SemanticError::InvalidFunctionTarget),
    }
}

fn resolve_type(ty: &Type, scope: &ScopeStack) -> Result<Type, SemanticError> {
    match ty {
        Type::Bit => Ok(Type::Bit),
        Type::Named(name) => match scope.lookup(name) {
            Some(Binding::Value(ty)) => Ok(ty.clone()),
            Some(Binding::Function { .. }) => Err(SemanticError::NotAValue(name.clone())),
            None => Err(SemanticError::UndefinedType(name.clone())),
        },
        Type::Reference(inner) => Ok(Type::Reference(Box::new(resolve_type(inner, scope)?))),
        Type::Collection(elements) => {
            let resolved = elements
                .iter()
                .map(|e| resolve_type(e, scope))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Type::Collection(resolved))
        }
    }
}

fn check_statement(stmt: &Statement, scope: &mut ScopeStack) -> Result<(), SemanticError> {
    match stmt {
        Statement::Binding(binding) => {
            let value = check_expression(&binding.value, scope)?;
            register_target(&binding.target, value, scope)
        }
        Statement::Expression(expr) => {
            check_expression(expr, scope)?;
            Ok(())
        }
    }
}

fn check_expression(expr: &Expression, scope: &mut ScopeStack) -> Result<Binding, SemanticError> {
    match expr {
        Expression::BitAllocation => Ok(Binding::Value(Type::Bit)),
        Expression::Identifier(name) => scope
            .lookup(name)
            .cloned()
            .ok_or_else(|| SemanticError::UndefinedLabel(name.clone())),
        Expression::Reference(name) => match scope.lookup(name) {
            Some(Binding::Value(Type::Reference(_))) => Err(SemanticError::NotARef(name.clone())),
            Some(Binding::Value(inner)) => {
                Ok(Binding::Value(Type::Reference(Box::new(inner.clone()))))
            }
            Some(Binding::Function { .. }) => Err(SemanticError::NotARef(name.clone())),
            None => Err(SemanticError::UndefinedLabel(name.clone())),
        },
        Expression::Nand(nand) => {
            let left = binding_type(&check_expression(&nand.left, scope)?)?;
            let right = binding_type(&check_expression(&nand.right, scope)?)?;
            if left != Type::Bit {
                return Err(SemanticError::NandRequiresBit(left));
            }
            if right != Type::Bit {
                return Err(SemanticError::NandRequiresBit(right));
            }
            Ok(Binding::Value(Type::Bit))
        }
        Expression::Branch(branch) => {
            let control = binding_type(&check_expression(&branch.control, scope)?)?;
            if control != Type::Bit {
                return Err(SemanticError::BranchControlNotBit(control));
            }
            let primary = binding_type(&check_block(&branch.primary_branch, scope)?)?;
            let secondary = binding_type(&check_block(&branch.secondary_branch, scope)?)?;
            if primary != secondary {
                return Err(SemanticError::BranchTypeMismatch(primary, secondary));
            }
            Ok(Binding::Value(primary))
        }
        Expression::FunctionCall(call) => {
            let (param_tys, return_ty) = match scope.lookup(&call.callee) {
                Some(Binding::Function {
                    parameters,
                    return_type,
                }) => (parameters.clone(), return_type.clone()),
                Some(Binding::Value(_)) => {
                    return Err(SemanticError::NotAFunction(call.callee.clone()));
                }
                None => return Err(SemanticError::UndefinedLabel(call.callee.clone())),
            };
            if call.arguments.len() != param_tys.len() {
                return Err(SemanticError::ArgumentCountMismatch {
                    label: call.callee.clone(),
                    expected: param_tys.len(),
                    found: call.arguments.len(),
                });
            }
            for (index, (arg, expected)) in call.arguments.iter().zip(param_tys.iter()).enumerate()
            {
                let actual = binding_type(&check_expression(arg, scope)?)?;
                let actual = match (&actual, expected) {
                    (Type::Reference(inner), expected) if inner.as_ref() == expected => {
                        inner.as_ref().clone()
                    }
                    _ => actual,
                };
                if actual != *expected {
                    return Err(SemanticError::ArgumentTypeMismatch {
                        label: call.callee.clone(),
                        index,
                        expected: expected.clone(),
                        found: actual,
                    });
                }
            }
            Ok(Binding::Value(return_ty))
        }
        Expression::ExternCall(_) => Ok(Binding::Value(Type::Bit)),
        Expression::Collection(elements) => {
            let tys = elements
                .iter()
                .map(|e| binding_type(&check_expression(e, scope)?))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Binding::Value(Type::Collection(tys)))
        }
        Expression::FunctionDefinition(func) => check_function_definition(func, scope),
    }
}

fn check_function_definition(
    func: &FunctionDefinition,
    scope: &mut ScopeStack,
) -> Result<Binding, SemanticError> {
    let mut param_tys = Vec::with_capacity(func.parameters.len());
    for p in &func.parameters {
        param_tys.push(resolve_type(&p.r#type, scope)?);
    }
    let return_ty = resolve_type(&func.return_type, scope)?;
    if matches!(return_ty, Type::Reference(_)) {
        return Err(SemanticError::FunctionReturnsReference(return_ty));
    }

    scope.push();
    for (p, ty) in func.parameters.iter().zip(param_tys.iter()) {
        scope.define(p.name.clone(), Binding::Value(ty.clone()))?;
    }

    for stmt in &func.body.statements {
        check_statement(stmt, scope)?;
    }

    let body_ty = if let Some(expr) = &func.body.trailing_expression {
        if matches!(**expr, Expression::ExternCall(_)) {
            return_ty.clone()
        } else {
            binding_type(&check_expression(expr, scope)?)?
        }
    } else {
        Type::Collection(Vec::new())
    };
    scope.pop();

    if body_ty != return_ty {
        return Err(SemanticError::TypeMismatch {
            expected: return_ty,
            found: body_ty,
        });
    }

    Ok(Binding::Function {
        parameters: param_tys,
        return_type: return_ty,
    })
}

fn check_block(block: &Block, scope: &mut ScopeStack) -> Result<Binding, SemanticError> {
    scope.push();
    for stmt in &block.statements {
        check_statement(stmt, scope)?;
    }
    let result = match &block.trailing_expression {
        Some(expr) => check_expression(expr, scope)?,
        None => Binding::Value(Type::Collection(Vec::new())),
    };
    scope.pop();
    Ok(result)
}

fn register_target(
    target: &Target,
    value: Binding,
    scope: &mut ScopeStack,
) -> Result<(), SemanticError> {
    match target {
        Target::Identifier(ident) => {
            if matches!(
                scope.lookup(&ident.name),
                Some(Binding::Value(Type::Reference(_)))
            ) {
                return write_through_reference(&ident.name, value, scope);
            }

            if let Some(declared) = &ident.r#type {
                let resolved = resolve_type(declared, scope)?;
                let actual = binding_type(&value)?;
                if resolved != actual {
                    return Err(SemanticError::TypeMismatch {
                        expected: resolved,
                        found: actual,
                    });
                }
            }
            scope.define(ident.name.clone(), value)
        }
        Target::Dereference(name) => write_through_reference(name, value, scope),
        Target::Collection(targets) => {
            let elements = match value {
                Binding::Value(Type::Collection(elements)) => elements,
                Binding::Value(other) => return Err(SemanticError::UnbindNonCollection(other)),
                Binding::Function { .. } => return Err(SemanticError::ExpectedValueGotFunction),
            };
            if targets.len() != elements.len() {
                return Err(SemanticError::UnbindArityMismatch {
                    expected: elements.len(),
                    found: targets.len(),
                });
            }
            for (t, ty) in targets.iter().zip(elements) {
                register_target(t, Binding::Value(ty), scope)?;
            }
            Ok(())
        }
    }
}

fn write_through_reference(
    name: &str,
    value: Binding,
    scope: &ScopeStack,
) -> Result<(), SemanticError> {
    let pointee = match scope.lookup(name) {
        Some(Binding::Value(Type::Reference(inner))) => inner.as_ref().clone(),
        Some(_) => return Err(SemanticError::NotARef(name.to_string())),
        None => return Err(SemanticError::UndefinedLabel(name.to_string())),
    };
    let actual = binding_type(&value)?;
    if pointee != actual {
        return Err(SemanticError::TypeMismatch {
            expected: pointee,
            found: actual,
        });
    }
    Ok(())
}

fn binding_type(binding: &Binding) -> Result<Type, SemanticError> {
    match binding {
        Binding::Value(ty) => Ok(ty.clone()),
        Binding::Function { .. } => Err(SemanticError::ExpectedValueGotFunction),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;
    use crate::tokenizer::Tokenizer;

    fn analyze_source(src: &str) -> Result<(), SemanticError> {
        let program = Parser::new(Tokenizer::new(src)).parse_program().unwrap();
        analyze(&program)
    }

    #[test]
    fn test_valid_minimal_program() {
        analyze_source(
            "
            main = () BIT {
                BIT
            }
            ",
        )
        .unwrap();
    }

    #[test]
    fn test_valid_canonical_program() {
        analyze_source(
            "
            seed = BIT
            ONE: BIT = NAND(seed, NAND(seed, seed))
            ZERO: BIT = NAND(ONE, ONE)
            U2 = [BIT, BIT]
            PAIR = [BIT, BIT]
            cast.u2.to.pair = (src: U2) PAIR {
                [b0, b1] = src
                [b0, b1]
            }
            not = (a: BIT) BIT {
                NAND(a, a)
            }
            xor = (a: BIT, b: BIT) BIT {
                n = NAND(a, b)
                NAND(NAND(a, n), NAND(b, n))
            }
            toggle = (target: *BIT) [] {
                target = not(target)
                []
            }
            choose = (flag: BIT, opt.a: U2, opt.b: U2) U2 {
                BRANCH (flag) {
                    opt.a
                } {
                    opt.b
                }
            }
            alloc.slot = () U2 {
                EXTERN(OP.ALLOC)
            }
            main = () BIT {
                BIT
            }
            ",
        )
        .unwrap();
    }

    #[test]
    fn test_undefined_label() {
        assert_eq!(
            analyze_source("main = () BIT { missing }"),
            Err(SemanticError::UndefinedLabel("missing".to_string()))
        );
    }

    #[test]
    fn test_duplicate_definition() {
        assert_eq!(
            analyze_source(
                "
                x: BIT = BIT
                x: BIT = BIT
                main = () BIT { BIT }
                "
            ),
            Err(SemanticError::DuplicateLabel("x".to_string()))
        );
    }

    #[test]
    fn test_type_mismatch_in_binding() {
        assert_eq!(
            analyze_source(
                "
                ONE: BIT = [BIT, BIT]
                main = () BIT { BIT }
                "
            ),
            Err(SemanticError::TypeMismatch {
                expected: Type::Bit,
                found: Type::Collection(vec![Type::Bit, Type::Bit]),
            })
        );
    }

    #[test]
    fn test_nand_requires_bit() {
        assert_eq!(
            analyze_source(
                "
                U2 = [BIT, BIT]
                one: U2 = [BIT, BIT]
                main = () BIT { NAND(one, one) }
                "
            ),
            Err(SemanticError::NandRequiresBit(Type::Collection(vec![
                Type::Bit,
                Type::Bit
            ])))
        );
    }

    #[test]
    fn test_branch_type_mismatch() {
        assert_eq!(
            analyze_source(
                "
                f = (flag: BIT) [] {
                    BRANCH(flag) { BIT } { [BIT, BIT] }
                }
                main = () BIT { BIT }
                "
            ),
            Err(SemanticError::BranchTypeMismatch(
                Type::Bit,
                Type::Collection(vec![Type::Bit, Type::Bit]),
            ))
        );
    }

    #[test]
    fn test_branch_control_not_bit() {
        assert_eq!(
            analyze_source(
                "
                U2 = [BIT, BIT]
                one: U2 = [BIT, BIT]
                f = () [] {
                    BRANCH(one) { BIT } { BIT }
                }
                main = () BIT { BIT }
                "
            ),
            Err(SemanticError::BranchControlNotBit(Type::Collection(vec![
                Type::Bit,
                Type::Bit
            ])))
        );
    }

    #[test]
    fn test_argument_count_mismatch() {
        assert_eq!(
            analyze_source(
                "
                f = (a: BIT) BIT { a }
                main = () BIT { f(BIT, BIT) }
                "
            ),
            Err(SemanticError::ArgumentCountMismatch {
                label: "f".to_string(),
                expected: 1,
                found: 2,
            })
        );
    }

    #[test]
    fn test_argument_type_mismatch() {
        assert_eq!(
            analyze_source(
                "
                U2 = [BIT, BIT]
                arg: U2 = [BIT, BIT]
                f = (a: BIT) BIT { a }
                main = () BIT { f(arg) }
                "
            ),
            Err(SemanticError::ArgumentTypeMismatch {
                label: "f".to_string(),
                index: 0,
                expected: Type::Bit,
                found: Type::Collection(vec![Type::Bit, Type::Bit]),
            })
        );
    }

    #[test]
    fn test_function_returns_reference() {
        assert_eq!(
            analyze_source(
                "
                f = (a: BIT) *BIT { a }
                main = () BIT { BIT }
                "
            ),
            Err(SemanticError::FunctionReturnsReference(Type::Reference(
                Box::new(Type::Bit)
            )))
        );
    }

    #[test]
    fn test_invalid_main_missing() {
        assert_eq!(
            analyze_source("x: BIT = BIT"),
            Err(SemanticError::InvalidMainSignature)
        );
    }

    #[test]
    fn test_invalid_main_with_params() {
        assert_eq!(
            analyze_source(
                "
                main = (x: BIT) BIT { x }
                "
            ),
            Err(SemanticError::InvalidMainSignature)
        );
    }

    #[test]
    fn test_invalid_main_wrong_return_type() {
        assert_eq!(
            analyze_source(
                "
                main = () [] { [] }
                "
            ),
            Err(SemanticError::InvalidMainSignature)
        );
    }

    #[test]
    fn test_unbind_arity_mismatch() {
        assert_eq!(
            analyze_source(
                "
                main = () BIT {
                    [a, b, c] = [BIT, BIT]
                    BIT
                }
                "
            ),
            Err(SemanticError::UnbindArityMismatch {
                expected: 2,
                found: 3,
            })
        );
    }

    #[test]
    fn test_unbind_non_collection() {
        assert_eq!(
            analyze_source(
                "
                main = () BIT {
                    [a, b] = BIT
                    BIT
                }
                "
            ),
            Err(SemanticError::UnbindNonCollection(Type::Bit))
        );
    }

    #[test]
    fn test_cannot_create_reference_to_reference() {
        assert_eq!(
            analyze_source(
                "
                main = () BIT {
                    state: BIT = BIT
                    ref: *BIT = *state
                    *ref
                    BIT
                }
                "
            ),
            Err(SemanticError::NotARef("ref".to_string()))
        );
    }

    #[test]
    fn test_not_a_function() {
        assert_eq!(
            analyze_source(
                "
                x: BIT = BIT
                main = () BIT { x(BIT) }
                "
            ),
            Err(SemanticError::NotAFunction("x".to_string()))
        );
    }

    #[test]
    fn test_hoisting_functions() {
        analyze_source(
            "
            main = () BIT {
                helper(BIT)
            }
            helper = (a: BIT) BIT { a }
            ",
        )
        .unwrap();
    }
}
