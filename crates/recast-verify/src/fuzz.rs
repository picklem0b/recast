//! V3 differential fuzzing.
//!
//! The fuzzer takes the calls a case already defines, mutates their arguments
//! through the TIR type of each parameter, and runs a second generated driver
//! on both implementations. Inputs are decoded from the TIR types, so an array
//! keeps its length, a record keeps its fields and a string keeps its
//! terminator; only values change. The seed and the generated corpus are
//! recorded, so a run is reproducible.
//!
//! This is deterministic mutation fuzzing, not coverage-guided fuzzing. The
//! report says so: coverage feedback is not implemented in this build.

use std::collections::BTreeMap;
use std::path::Path;

use recast_ir::{Program, Type};
use serde::Serialize;
use serde_json::Value;

use crate::{
    compile_c, compile_rust_driver, driver, run as run_binary, write, Case, Error, Options,
    SWEEP_SEED,
};

/// One function's V3 result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FuzzReport {
    pub function: String,
    pub executions: u64,
    pub seed: u64,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// The V3 result, plus the driver and C baseline mutation testing reuses.
pub struct FuzzRun {
    pub reports: Vec<FuzzReport>,
    /// The Rust driver the fuzzer compiled against the generated crate.
    pub driver: std::path::PathBuf,
    /// What the C original printed for the same inputs.
    pub expected: String,
}

/// Run V3 for `case`, mutating its calls `executions` times. `None` means the
/// case defined no calls (or fuzzing was disabled), so there is nothing to run.
pub fn run(
    options: &Options,
    program: &Program,
    case: &Case,
    source: &Path,
    executions: u64,
) -> Result<Option<FuzzRun>, Error> {
    if executions == 0 || case.calls.is_empty() {
        return Ok(None);
    }
    let seed = SWEEP_SEED ^ 0x9E37_79B9_7F4A_7C15;
    let mut random = Random::new(seed);
    let mut calls = Vec::new();
    let mut per_function: BTreeMap<String, u64> = BTreeMap::new();
    for index in 0..executions {
        let seed_call = &case.calls[index as usize % case.calls.len()];
        let mut call = seed_call.clone();
        if let Some(function) = program.function_named(&call.function) {
            let policies = policies(function);
            let params = &function.signature.params;
            let group = recast_ir::slice::slice_group(program, params);
            let mut used = 0;
            for (position, param) in params.iter().enumerate() {
                // A slice's length is derived from the slice by the driver; it
                // is not one of the case's argument values.
                if group.as_ref().is_some_and(|group| group.length == position) {
                    continue;
                }
                let Some(argument) = call.args.get_mut(used) else {
                    break;
                };
                used += 1;
                let policy = policies.get(position).copied().unwrap_or(Policy::Plain);
                if let Some(next) = mutate(program, param.ty, argument, policy, &mut random) {
                    *argument = next;
                }
            }
        }
        *per_function.entry(call.function.clone()).or_insert(0) += 1;
        calls.push(call);
    }

    let mutated = Case {
        schema: "recast-case/1".to_string(),
        calls,
        sweeps: Vec::new(),
        test_command: None,
    };
    let drivers = driver::calls(program, &mutated, source)?;
    let directory = options.work.join("v3");
    std::fs::create_dir_all(&directory).map_err(|error| Error::Io {
        path: directory.clone(),
        message: error.to_string(),
    })?;
    let c_path = directory.join("fuzz.c");
    let rust_path = directory.join("fuzz.rs");
    write(&c_path, &drivers.c)?;
    write(&rust_path, &drivers.rust)?;
    let c_binary = directory.join("fuzz-c");
    let rust_binary = directory.join("fuzz-rust");
    compile_c(options, &c_path, &c_binary)?;
    compile_rust_driver(
        options,
        &rust_path,
        &options.work.join("libgenerated.rlib"),
        &rust_binary,
    )?;
    let c_run = run_binary(&c_binary)?;
    let rust_run = run_binary(&rust_binary)?;

    let corpus = serde_json::json!({
        "schema": "recast-fuzz-corpus/1",
        "seed": seed,
        "executions": executions,
        "calls": &mutated.calls,
    });
    let corpus = serde_json::to_string_pretty(&corpus)
        .map_err(|error| Error::Unsupported(error.to_string()))?;
    write(&directory.join("corpus.json"), &format!("{corpus}\n"))?;

    let passed = c_run == rust_run;
    let failure = (!passed).then(|| {
        format!(
            "fuzz outputs differ\n--- C ---\n{}\n--- Rust ---\n{}",
            truncate(&c_run, 2000),
            truncate(&rust_run, 2000)
        )
    });
    let reports = per_function
        .into_iter()
        .map(|(function, executions)| FuzzReport {
            function,
            executions,
            seed,
            passed,
            failure: failure.clone(),
        })
        .collect::<Vec<_>>();
    let result = serde_json::json!({
        "schema": "recast-fuzz-result/1",
        "seed": seed,
        "executions": executions,
        "functions": reports,
    });
    let result = serde_json::to_string_pretty(&result)
        .map_err(|error| Error::Unsupported(error.to_string()))?;
    write(&directory.join("result.json"), &format!("{result}\n"))?;
    Ok(Some(FuzzRun {
        reports,
        driver: rust_path.clone(),
        expected: c_run,
    }))
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

struct Random(u64);

impl Random {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// What the body does with one parameter, so a mutation does not turn the
/// differential into a comparison of two undefined behaviors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Policy {
    Plain,
    /// The parameter is the right operand of `<<` or `>>`: keep the count in
    /// `0..bits`.
    ShiftCount,
    /// The parameter is the right operand of `/` or `%`: never generate zero.
    NonZero,
    /// The parameter controls a loop: keep the iteration count small enough
    /// that a generated driver finishes inside its time budget.
    LoopBound,
}

fn policies(function: &recast_ir::Function) -> Vec<Policy> {
    let mut policies = vec![Policy::Plain; function.signature.params.len()];
    let Some(body) = &function.body else {
        return policies;
    };
    let mut mark = |name: &str, policy: Policy| {
        if let Some(position) = function
            .signature
            .params
            .iter()
            .position(|param| param.name.as_deref() == Some(name))
        {
            // The specific guards matter more than the loop bound: a divisor
            // must stay nonzero even when it also bounds a loop.
            if policy != Policy::LoopBound || policies[position] == Policy::Plain {
                policies[position] = policy;
            }
        }
    };
    visit_block(body, &mut mark);
    policies
}

fn visit_block(block: &recast_ir::Block, mark: &mut impl FnMut(&str, Policy)) {
    for stmt in &block.stmts {
        visit_stmt(stmt, mark);
    }
}

fn visit_stmt(stmt: &recast_ir::Stmt, mark: &mut impl FnMut(&str, Policy)) {
    use recast_ir::StmtKind;
    match &stmt.kind {
        StmtKind::Decl { init, .. } => {
            if let Some(init) = init {
                visit_expr(init, mark);
            }
        }
        StmtKind::Expr(expr) => visit_expr(expr, mark),
        StmtKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            visit_expr(cond, mark);
            visit_block(then_branch, mark);
            if let Some(else_branch) = else_branch {
                visit_block(else_branch, mark);
            }
        }
        StmtKind::While { cond, body } => {
            mark_locals(cond, mark);
            visit_expr(cond, mark);
            visit_block(body, mark);
        }
        StmtKind::For {
            init,
            cond,
            step,
            body,
        } => {
            if let Some(init) = init {
                visit_stmt(init, mark);
            }
            if let Some(cond) = cond {
                mark_locals(cond, mark);
                visit_expr(cond, mark);
            }
            if let Some(step) = step {
                visit_expr(step, mark);
            }
            visit_block(body, mark);
        }
        StmtKind::DoWhile { body, cond } => {
            visit_block(body, mark);
            mark_locals(cond, mark);
            visit_expr(cond, mark);
        }
        StmtKind::Switch { scrutinee, cases } => {
            visit_expr(scrutinee, mark);
            for case in cases {
                visit_block(&case.body, mark);
            }
        }
        StmtKind::Return(Some(expr)) => visit_expr(expr, mark),
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Empty => {}
    }
}

fn visit_expr(expr: &recast_ir::Expr, mark: &mut impl FnMut(&str, Policy)) {
    use recast_ir::ExprKind;
    match &expr.kind {
        ExprKind::Binary { op, lhs, rhs } => {
            let policy = match op.as_str() {
                "<<" | ">>" => Some(Policy::ShiftCount),
                "/" | "%" => Some(Policy::NonZero),
                _ => None,
            };
            if let (Some(policy), Some(name)) = (policy, local_name(rhs)) {
                mark(name, policy);
            }
            visit_expr(lhs, mark);
            visit_expr(rhs, mark);
        }
        ExprKind::Assign { op, lhs, rhs } => {
            let policy = match op.as_str() {
                "<<=" | ">>=" => Some(Policy::ShiftCount),
                "/=" | "%=" => Some(Policy::NonZero),
                _ => None,
            };
            if let (Some(policy), Some(name)) = (policy, local_name(rhs)) {
                mark(name, policy);
            }
            visit_expr(lhs, mark);
            visit_expr(rhs, mark);
        }
        ExprKind::Member { base, .. } => visit_expr(base, mark),
        ExprKind::Cond {
            cond,
            then_branch,
            else_branch,
        } => {
            visit_expr(cond, mark);
            visit_expr(then_branch, mark);
            visit_expr(else_branch, mark);
        }
        ExprKind::InitList(items) => {
            for item in items {
                visit_expr(item, mark);
            }
        }
        ExprKind::Unary { operand, .. } => visit_expr(operand, mark),
        ExprKind::Index { base, index } => {
            visit_expr(base, mark);
            visit_expr(index, mark);
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                visit_expr(arg, mark);
            }
        }
        ExprKind::Cast { expr, .. } | ExprKind::Paren(expr) => visit_expr(expr, mark),
        ExprKind::IntLit(_)
        | ExprKind::FloatLit(_)
        | ExprKind::StringLit(_)
        | ExprKind::CharLit(_)
        | ExprKind::BoolLit(_)
        | ExprKind::Null
        | ExprKind::Local(_)
        | ExprKind::Global(_)
        | ExprKind::EnumConst { .. }
        | ExprKind::Unsupported(_) => {}
    }
}

/// Mark every local the expression mentions as a loop bound.
fn mark_locals(expr: &recast_ir::Expr, mark: &mut impl FnMut(&str, Policy)) {
    use recast_ir::ExprKind;
    match &expr.kind {
        ExprKind::Local(name) => mark(name, Policy::LoopBound),
        ExprKind::Member { base, .. } => mark_locals(base, mark),
        ExprKind::Cond {
            cond,
            then_branch,
            else_branch,
        } => {
            mark_locals(cond, mark);
            mark_locals(then_branch, mark);
            mark_locals(else_branch, mark);
        }
        ExprKind::InitList(items) => {
            for item in items {
                mark_locals(item, mark);
            }
        }
        ExprKind::Unary { operand, .. } => mark_locals(operand, mark),
        ExprKind::Binary { lhs, rhs, .. } | ExprKind::Assign { lhs, rhs, .. } => {
            mark_locals(lhs, mark);
            mark_locals(rhs, mark);
        }
        ExprKind::Index { base, index } => {
            mark_locals(base, mark);
            mark_locals(index, mark);
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                mark_locals(arg, mark);
            }
        }
        ExprKind::Cast { expr, .. } | ExprKind::Paren(expr) => mark_locals(expr, mark),
        ExprKind::IntLit(_)
        | ExprKind::FloatLit(_)
        | ExprKind::StringLit(_)
        | ExprKind::CharLit(_)
        | ExprKind::BoolLit(_)
        | ExprKind::Null
        | ExprKind::Global(_)
        | ExprKind::EnumConst { .. }
        | ExprKind::Unsupported(_) => {}
    }
}

/// The parameter name under any casts and parentheses on the right operand.
fn local_name(expr: &recast_ir::Expr) -> Option<&str> {
    use recast_ir::ExprKind;
    match &expr.kind {
        ExprKind::Local(name) => Some(name),
        ExprKind::Cast { expr, .. } | ExprKind::Paren(expr) => local_name(expr),
        _ => None,
    }
}

fn mutate(
    program: &Program,
    ty: recast_ir::TypeId,
    value: &Value,
    policy: Policy,
    random: &mut Random,
) -> Option<Value> {
    match program.types.canonical(ty)? {
        Type::Bool => Some(Value::Bool(random.next() & 1 == 0)),
        Type::Char { signed } => Some(number(*signed, 8, policy, value, random)),
        Type::Int { bits, signed, .. } => Some(number(*signed, *bits, policy, value, random)),
        Type::Enum { name } => {
            let underlying = program.enum_named(name)?.underlying;
            mutate(program, underlying, value, policy, random)
        }
        Type::Array { elem, len } => {
            let items = value.as_array()?;
            if items.len() != usize::try_from(*len).ok()? {
                return None;
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(mutate(program, *elem, item, Policy::Plain, random)?);
            }
            Some(Value::Array(out))
        }
        Type::Record { name } => {
            let object = value.as_object()?;
            let record = program.record(name)?;
            let mut out = serde_json::Map::new();
            for field in &record.fields {
                let field_value = object.get(&field.name)?;
                out.insert(
                    field.name.clone(),
                    mutate(program, field.ty, field_value, Policy::Plain, random)?,
                );
            }
            Some(Value::Object(out))
        }
        Type::Pointer { pointee, .. } => {
            // The driver treats an array behind a pointer as a slice and a
            // string as a NUL-terminated range. Mutate the elements, never the
            // shape: a scalar here would turn a slice argument into a length
            // error before either implementation runs.
            if let Some(items) = value.as_array() {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(mutate(program, *pointee, item, Policy::Plain, random)?);
                }
                return Some(Value::Array(out));
            }
            if value.is_string() {
                return Some(value.clone());
            }
            mutate(program, *pointee, value, policy, random)
        }
        // A string's length is part of the case contract; leave it alone.
        Type::Void | Type::Float { .. } | Type::Named { .. } | Type::Unknown { .. } => {
            Some(value.clone())
        }
    }
}

fn number(signed: bool, bits: u8, policy: Policy, current: &Value, random: &mut Random) -> Value {
    // A mutation stays in the seed value's neighborhood. Jumping to a random
    // full-width value would turn a loop count into a multi-billion-iteration
    // run and a factorial into signed overflow, comparing C's undefined
    // behavior with the translated policy. V4 is the rung that looks for UB.
    let delta = (random.next() % 7) as i64 - 3;
    if signed {
        let start = current.as_i64().unwrap_or(0);
        let value = match policy {
            Policy::ShiftCount => {
                // A C shift by `bits` or more is undefined; the translated
                // crate panics instead. Stay inside the defined range.
                let width = i64::from(bits.clamp(1, 64));
                start.saturating_add(delta).clamp(0, width - 1)
            }
            Policy::LoopBound => {
                let value = start.saturating_add(delta).clamp(0, 255);
                // A zero loop bound is finite for a counted loop but can make
                // a `while (n != 1)` transformation loop forever.
                if start > 0 && value == 0 {
                    1
                } else {
                    value
                }
            }
            Policy::NonZero | Policy::Plain => {
                let width = bits.clamp(1, 63);
                let min = -(1i64 << (width - 1));
                let max = (1i64 << (width - 1)) - 1;
                let value = start.saturating_add(delta).clamp(min, max);
                if policy == Policy::NonZero && value == 0 {
                    1
                } else {
                    value
                }
            }
        };
        Value::from(value)
    } else {
        let start = current
            .as_u64()
            .or_else(|| current.as_i64().map(|value| value as u64))
            .unwrap_or(0);
        let start = match policy {
            Policy::ShiftCount => start.min(u64::from(bits.clamp(1, 64)) - 1),
            Policy::LoopBound => start.min(255),
            Policy::NonZero | Policy::Plain => start,
        };
        let mut value = if delta >= 0 {
            start.wrapping_add(delta as u64)
        } else {
            start.wrapping_sub((-delta) as u64)
        };
        let bits = bits.min(64);
        if bits < 64 {
            value &= (1u64 << bits) - 1;
        }
        if policy == Policy::LoopBound && start > 0 && value == 0 {
            value = 1;
        }
        if policy == Policy::NonZero && value == 0 {
            value = 1;
        }
        Value::from(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recast_ir::Target;

    #[test]
    fn mutations_stay_in_the_seed_neighborhood() {
        let mut random = Random::new(7);
        for _ in 0..1000 {
            let value = number(true, 32, Policy::Plain, &Value::from(100), &mut random);
            let value = value.as_i64().expect("integer");
            assert!((97..=103).contains(&value), "{value}");
        }
    }

    #[test]
    fn guard_policies_keep_shift_counts_and_divisors_defined() {
        let mut random = Random::new(9);
        for _ in 0..1000 {
            let shift = number(true, 32, Policy::ShiftCount, &Value::from(1), &mut random);
            assert!((0..32).contains(&shift.as_u64().expect("shift count")));
            let divisor = number(true, 32, Policy::NonZero, &Value::from(1), &mut random);
            assert_ne!(divisor.as_i64(), Some(0));
            let divisor = number(false, 32, Policy::NonZero, &Value::from(1), &mut random);
            assert_ne!(divisor.as_u64(), Some(0));
            let bound = number(true, 32, Policy::LoopBound, &Value::from(120), &mut random);
            assert!((1..=255).contains(&bound.as_i64().expect("loop bound")));
        }
    }

    #[test]
    fn a_pointer_slice_mutation_keeps_its_elements() {
        let mut program = Program::new(Target::lp64("x86_64-unknown-linux-gnu"));
        let int = program.types.intern(Type::Int {
            bits: 32,
            signed: true,
            spelling: "int".to_string(),
        });
        let pointer = program.types.intern(Type::Pointer {
            pointee: int,
            is_const: false,
        });
        let mut random = Random::new(11);
        let value = serde_json::json!([1, 2, 3]);
        let mutated =
            mutate(&program, pointer, &value, Policy::Plain, &mut random).expect("slice mutation");
        let items = mutated.as_array().expect("an array stays an array");
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|item| item.is_i64()));

        let string = Value::String("hello".to_string());
        assert_eq!(
            mutate(&program, pointer, &string, Policy::Plain, &mut random),
            Some(string),
            "a string's shape and length are part of the case"
        );
    }

    #[test]
    fn a_record_mutation_keeps_every_field() {
        let mut program = Program::new(Target::lp64("x86_64-unknown-linux-gnu"));
        let int = program.types.intern(Type::Int {
            bits: 32,
            signed: true,
            spelling: "int".to_string(),
        });
        let record = program.types.intern(Type::Record {
            name: "Pair".to_string(),
        });
        program.records.push(recast_ir::RecordDef {
            id: recast_ir::ItemId::record("Pair"),
            name: "Pair".to_string(),
            kind: recast_ir::RecordKind::Struct,
            fields: vec![
                recast_ir::FieldDef {
                    name: "a".to_string(),
                    ty: int,
                },
                recast_ir::FieldDef {
                    name: "b".to_string(),
                    ty: int,
                },
            ],
            size: 8,
            align: 4,
            provenance: recast_ir::Provenance::synthesized(),
        });
        let mut random = Random::new(3);
        let value = serde_json::json!({"a": 1, "b": 2});
        let mutated =
            mutate(&program, record, &value, Policy::Plain, &mut random).expect("record mutation");
        assert!(mutated.get("a").is_some() && mutated.get("b").is_some());
    }
}
