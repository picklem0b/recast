//! Undefined-behavior detection for the M1 subset.
//!
//! M1 folds constant expressions and reports the cases where the operand values
//! are known:
//!
//! - `UB-001`: signed overflow in `+`, `-`, `*` or unary `-`.
//! - `UB-007`: a shift count outside `0..width`.
//! - `UB-008`: division or remainder by zero, and `MIN / -1`.
//!
//! The interval analysis from document 05 section 10 is the M2 replacement;
//! the rule identifiers and finding shape stay the same.

use std::collections::BTreeMap;

use recast_ir::{Block, Expr, ExprKind, Function, Program, Span, Stmt, StmtKind, Type, TypeId};

use crate::definite::int_width;
use crate::Finding;

/// Check one function and append findings.
pub fn check(function: &Function, program: &Program, findings: &mut Vec<Finding>) {
    let Some(body) = &function.body else {
        return;
    };
    let mut checker = Checker {
        function: &function.name,
        program,
        findings,
    };
    let mut constants = BTreeMap::new();
    checker.block(body, &mut constants, &function.provenance.span);
}

struct Checker<'a> {
    function: &'a str,
    program: &'a Program,
    findings: &'a mut Vec<Finding>,
}

impl Checker<'_> {
    fn report(&mut self, rule: &'static str, span: &Span, message: String) {
        self.findings
            .push(Finding::new(rule, self.function, span.clone(), message));
    }

    fn block(&mut self, block: &Block, constants: &mut BTreeMap<String, i128>, span: &Span) {
        for stmt in &block.stmts {
            self.stmt(stmt, constants, span);
        }
    }

    fn stmt(&mut self, stmt: &Stmt, constants: &mut BTreeMap<String, i128>, span: &Span) {
        let span = if stmt.provenance.span.line > 0 {
            &stmt.provenance.span
        } else {
            span
        };
        match &stmt.kind {
            StmtKind::Decl { name, ty, init } => {
                if let Some(init) = init {
                    self.expr(init, constants, span);
                    match const_eval(init, self.program, constants) {
                        Some(value) => {
                            let value = truncate(value, *ty, self.program);
                            constants.insert(name.clone(), value);
                        }
                        None => {
                            constants.remove(name);
                        }
                    }
                } else {
                    constants.remove(name);
                }
            }
            StmtKind::Expr(expr) => {
                self.expr(expr, constants, span);
                invalidate(expr, constants);
            }
            StmtKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.expr(cond, constants, span);
                let entry = constants.clone();
                let mut then_constants = entry.clone();
                self.block(then_branch, &mut then_constants, span);
                let mut else_constants = entry.clone();
                if let Some(else_branch) = else_branch {
                    self.block(else_branch, &mut else_constants, span);
                }
                // Keep only constants that survive both branches unchanged.
                constants.retain(|name, value| {
                    then_constants.get(name) == Some(value)
                        && else_constants.get(name) == Some(value)
                });
            }
            StmtKind::While { cond, body } => {
                self.expr(cond, constants, span);
                let mut body_constants = constants.clone();
                self.block(body, &mut body_constants, span);
                invalidate_block_writes(body, constants);
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some(init) = init {
                    self.stmt(init, constants, span);
                }
                if let Some(cond) = cond {
                    self.expr(cond, constants, span);
                }
                let mut body_constants = constants.clone();
                self.block(body, &mut body_constants, span);
                if let Some(step) = step {
                    self.expr(step, &body_constants, span);
                }
                invalidate_block_writes(body, constants);
            }
            StmtKind::DoWhile { body, cond } => {
                let mut body_constants = constants.clone();
                self.block(body, &mut body_constants, span);
                self.expr(cond, &body_constants, span);
                invalidate_block_writes(body, constants);
            }
            StmtKind::Switch { scrutinee, cases } => {
                self.expr(scrutinee, constants, span);
                for case in cases {
                    let mut case_constants = constants.clone();
                    self.block(&case.body, &mut case_constants, span);
                }
                for case in cases {
                    invalidate_block_writes(&case.body, constants);
                }
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.expr(value, constants, span);
                }
            }
            StmtKind::Break | StmtKind::Continue | StmtKind::Empty => {}
        }
    }

    fn expr(&mut self, expr: &Expr, constants: &BTreeMap<String, i128>, span: &Span) {
        match &expr.kind {
            ExprKind::Binary { op, lhs, rhs } => {
                self.expr(lhs, constants, span);
                self.expr(rhs, constants, span);
                let left = const_eval(lhs, self.program, constants);
                let right = const_eval(rhs, self.program, constants);
                match op.as_str() {
                    "/" | "%" => {
                        if right == Some(0) {
                            self.report(
                                "UB-008",
                                span,
                                format!("{op} by zero is undefined behavior"),
                            );
                        }
                        if let (Some(left), Some(right)) = (left, right) {
                            if right == -1 {
                                if let Some(min) = signed_min(self.program, lhs.ty) {
                                    if left == min {
                                        self.report(
                                            "UB-008",
                                            span,
                                            format!(
                                                "{op} of the minimum signed value by -1 overflows"
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    "<<" | ">>" => {
                        if let (Some(count), Some(width)) = (right, int_width(self.program, lhs.ty))
                        {
                            if count < 0 || count >= i128::from(width) {
                                self.report(
                                    "UB-007",
                                    span,
                                    format!(
                                        "shift count {count} is outside 0..{width} for a {width}-bit value"
                                    ),
                                );
                            }
                        }
                        if op == "<<" {
                            if let (Some(value), Some(min)) =
                                (left, signed_min(self.program, lhs.ty))
                            {
                                if value < 0 {
                                    self.report(
                                        "UB-001",
                                        span,
                                        format!(
                                            "left shift of negative value {value} is undefined"
                                        ),
                                    );
                                }
                                let _ = min;
                            }
                        }
                    }
                    "+" | "-" | "*" => {
                        if let (Some(left), Some(right)) = (left, right) {
                            if let Some((min, max)) = signed_range(self.program, lhs.ty) {
                                let folded = match op.as_str() {
                                    "+" => left.checked_add(right),
                                    "-" => left.checked_sub(right),
                                    _ => left.checked_mul(right),
                                };
                                match folded {
                                    Some(value) if value < min || value > max => {
                                        self.report(
                                            "UB-001",
                                            span,
                                            format!(
                                                "signed {op} overflows: {left} {op} {right} = {value}, range {min}..={max}"
                                            ),
                                        );
                                    }
                                    None => self.report(
                                        "UB-001",
                                        span,
                                        format!("signed {op} of {left} and {right} overflows"),
                                    ),
                                    Some(_) => {}
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            ExprKind::Unary { op, operand, .. } => {
                self.expr(operand, constants, span);
                if op == "-" {
                    if let (Some(value), Some(min)) = (
                        const_eval(operand, self.program, constants),
                        signed_min(self.program, expr.ty),
                    ) {
                        if value == min {
                            self.report(
                                "UB-001",
                                span,
                                format!("negating the minimum signed value {value} overflows"),
                            );
                        }
                    }
                }
            }
            ExprKind::Assign { lhs, rhs, .. } => {
                self.expr(lhs, constants, span);
                self.expr(rhs, constants, span);
            }
            ExprKind::Index { base, index } => {
                self.expr(base, constants, span);
                self.expr(index, constants, span);
            }
            ExprKind::Member { base, .. } => self.expr(base, constants, span),
            ExprKind::Cond {
                cond,
                then_branch,
                else_branch,
            } => {
                self.expr(cond, constants, span);
                self.expr(then_branch, constants, span);
                self.expr(else_branch, constants, span);
            }
            ExprKind::InitList(elems) => {
                for elem in elems {
                    self.expr(elem, constants, span);
                }
            }
            ExprKind::Call { args, .. } => {
                for arg in args {
                    self.expr(arg, constants, span);
                }
            }
            ExprKind::Cast { expr, .. } | ExprKind::Paren(expr) => self.expr(expr, constants, span),
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
}

fn invalidate(expr: &Expr, constants: &mut BTreeMap<String, i128>) {
    if let ExprKind::Assign { lhs, .. } = &expr.kind {
        if let Some(name) = crate::cfg::root_local(lhs) {
            constants.remove(name);
        }
    }
    match &expr.kind {
        ExprKind::Unary { op, operand, .. } => {
            if op == "++" || op == "--" {
                if let Some(name) = crate::cfg::root_local(operand) {
                    constants.remove(name);
                }
            }
            invalidate(operand, constants);
        }
        ExprKind::Binary { lhs, rhs, .. } | ExprKind::Assign { lhs, rhs, .. } => {
            invalidate(lhs, constants);
            invalidate(rhs, constants);
        }
        ExprKind::Index { base, index } => {
            invalidate(base, constants);
            invalidate(index, constants);
        }
        ExprKind::Member { base, .. } => invalidate(base, constants),
        ExprKind::Cond {
            cond,
            then_branch,
            else_branch,
        } => {
            invalidate(cond, constants);
            invalidate(then_branch, constants);
            invalidate(else_branch, constants);
        }
        ExprKind::InitList(elems) => {
            for elem in elems {
                invalidate(elem, constants);
            }
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                invalidate(arg, constants);
            }
        }
        ExprKind::Cast { expr, .. } | ExprKind::Paren(expr) => invalidate(expr, constants),
        _ => {}
    }
}

fn invalidate_block_writes(block: &Block, constants: &mut BTreeMap<String, i128>) {
    for stmt in &block.stmts {
        match &stmt.kind {
            StmtKind::Decl { name, .. } => {
                constants.remove(name);
            }
            StmtKind::Expr(expr) => invalidate(expr, constants),
            StmtKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                invalidate(cond, constants);
                invalidate_block_writes(then_branch, constants);
                if let Some(else_branch) = else_branch {
                    invalidate_block_writes(else_branch, constants);
                }
            }
            StmtKind::While { cond, body } => {
                invalidate(cond, constants);
                invalidate_block_writes(body, constants);
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some(init) = init {
                    if let StmtKind::Decl { name, .. } = &init.kind {
                        constants.remove(name);
                    }
                }
                if let Some(cond) = cond {
                    invalidate(cond, constants);
                }
                if let Some(step) = step {
                    invalidate(step, constants);
                }
                invalidate_block_writes(body, constants);
            }
            StmtKind::DoWhile { body, cond } => {
                invalidate_block_writes(body, constants);
                invalidate(cond, constants);
            }
            StmtKind::Switch { scrutinee, cases } => {
                invalidate(scrutinee, constants);
                for case in cases {
                    invalidate_block_writes(&case.body, constants);
                }
            }
            StmtKind::Return(Some(value)) => invalidate(value, constants),
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Empty => {}
        }
    }
}

/// Fold a constant expression, ignoring overflow beyond `i128`.
pub fn const_eval(
    expr: &Expr,
    program: &Program,
    constants: &BTreeMap<String, i128>,
) -> Option<i128> {
    match &expr.kind {
        ExprKind::IntLit(value) => Some(i128::from(*value)),
        ExprKind::CharLit(value) => Some(i128::from(*value)),
        ExprKind::BoolLit(value) => Some(i128::from(*value)),
        ExprKind::Local(name) => constants.get(name).copied(),
        ExprKind::EnumConst { value, .. } => Some(i128::from(*value)),
        ExprKind::Paren(inner) => const_eval(inner, program, constants),
        ExprKind::Cast { to, expr, .. } => {
            const_eval(expr, program, constants).map(|value| truncate(value, *to, program))
        }
        ExprKind::Unary { op, operand, .. } => {
            let value = const_eval(operand, program, constants)?;
            match op.as_str() {
                "-" => value.checked_neg(),
                "~" => {
                    let width = int_width(program, expr.ty)?;
                    let mask = mask(width);
                    Some((!value) & mask)
                }
                "!" => Some(i128::from(value == 0)),
                "+" => Some(value),
                _ => None,
            }
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let left = const_eval(lhs, program, constants)?;
            let right = const_eval(rhs, program, constants)?;
            let folded = match op.as_str() {
                "+" => left.checked_add(right),
                "-" => left.checked_sub(right),
                "*" => left.checked_mul(right),
                "/" if right != 0 => left.checked_div(right),
                "%" if right != 0 => left.checked_rem(right),
                "&" => Some(left & right),
                "|" => Some(left | right),
                "^" => Some(left ^ right),
                "<<" if (0..128).contains(&right) => left.checked_shl(right as u32),
                ">>" if (0..128).contains(&right) => Some(left >> right),
                "==" => Some(i128::from(left == right)),
                "!=" => Some(i128::from(left != right)),
                "<" => Some(i128::from(left < right)),
                "<=" => Some(i128::from(left <= right)),
                ">" => Some(i128::from(left > right)),
                ">=" => Some(i128::from(left >= right)),
                "&&" => Some(i128::from(left != 0 && right != 0)),
                "||" => Some(i128::from(left != 0 || right != 0)),
                _ => None,
            }?;
            Some(truncate(folded, expr.ty, program))
        }
        ExprKind::Cond {
            cond,
            then_branch,
            else_branch,
        } => {
            if const_eval(cond, program, constants)? != 0 {
                const_eval(then_branch, program, constants)
            } else {
                const_eval(else_branch, program, constants)
            }
        }
        _ => None,
    }
}

fn mask(width: u16) -> i128 {
    if width >= 128 {
        i128::MAX
    } else {
        (1i128 << width) - 1
    }
}

/// Truncate a value to the representation of `ty`, the way a C conversion
/// would.
pub fn truncate(value: i128, ty: TypeId, program: &Program) -> i128 {
    let Some(width) = int_width(program, ty) else {
        return value;
    };
    if width >= 128 {
        return value;
    }
    let masked = value & mask(width);
    if signed(program, ty) && (masked & (1i128 << (width - 1))) != 0 {
        masked - (1i128 << width)
    } else {
        masked
    }
}

fn signed(program: &Program, id: TypeId) -> bool {
    match program.types.canonical(id) {
        Some(Type::Int { signed, .. }) => *signed,
        Some(Type::Enum { name }) => program
            .enum_named(name)
            .and_then(|item| program.types.canonical(item.underlying))
            .is_some_and(|ty| matches!(ty, Type::Int { signed: true, .. })),
        Some(Type::Char { signed }) => *signed,
        _ => false,
    }
}

fn signed_range(program: &Program, id: TypeId) -> Option<(i128, i128)> {
    if !signed(program, id) {
        return None;
    }
    let width = int_width(program, id)?;
    if width == 0 || width > 127 {
        return None;
    }
    Some((-(1i128 << (width - 1)), (1i128 << (width - 1)) - 1))
}

fn signed_min(program: &Program, id: TypeId) -> Option<i128> {
    signed_range(program, id).map(|(min, _)| min)
}
