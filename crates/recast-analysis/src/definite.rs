//! Definite assignment (UB-005).
//!
//! The analysis is a forward must-analysis over the structured tree. It reports
//! reads of locals that are not assigned on every path that reaches them. A
//! statement that cannot fall through, such as `return`, `break` or `continue`,
//! ends the path it sits on, so the assignments of that path are not required
//! at the join. The analysis is deliberately conservative at loop joins: a
//! variable assigned in a loop body is not assumed assigned after the loop,
//! except for `do-while`, which runs its body at least once.

use std::collections::BTreeSet;

use recast_ir::{Block, Expr, ExprKind, Function, Span, Stmt, StmtKind, Type, TypeId};

use crate::Finding;

/// Check one function and append findings.
pub fn check(function: &Function, program: &recast_ir::Program, findings: &mut Vec<Finding>) {
    let Some(body) = &function.body else {
        return;
    };
    let mut checker = Checker {
        function: &function.name,
        program,
        findings,
    };
    let mut assigned = BTreeSet::new();
    for param in &function.signature.params {
        if let Some(name) = &param.name {
            assigned.insert(name.clone());
        }
    }
    let _ = checker.block(body, &mut assigned, &function.provenance.span);
}

struct Checker<'a> {
    function: &'a str,
    program: &'a recast_ir::Program,
    findings: &'a mut Vec<Finding>,
}

impl Checker<'_> {
    fn report(&mut self, span: &Span, message: String) {
        self.findings
            .push(Finding::new("UB-005", self.function, span.clone(), message));
    }

    /// Analyze the statements in order and report whether control can reach the
    /// end of the block. Statements after one that cannot fall through are
    /// unreachable, so they are not analyzed.
    fn block(&mut self, block: &Block, assigned: &mut BTreeSet<String>, span: &Span) -> bool {
        for stmt in &block.stmts {
            if !self.stmt(stmt, assigned, span) {
                return false;
            }
        }
        true
    }

    /// Analyze one statement and report whether it can fall through to the
    /// statement that follows it.
    fn stmt(&mut self, stmt: &Stmt, assigned: &mut BTreeSet<String>, span: &Span) -> bool {
        let span = if stmt.provenance.span.line > 0 {
            &stmt.provenance.span
        } else {
            span
        };
        match &stmt.kind {
            StmtKind::Decl { name, init, .. } => {
                if let Some(init) = init {
                    self.expr(init, assigned, span);
                    assigned.insert(name.clone());
                } else {
                    // `int x;` leaves x indeterminate in C.
                    assigned.remove(name);
                }
                true
            }
            StmtKind::Expr(expr) => {
                self.expr(expr, assigned, span);
                apply_defs(expr, assigned);
                true
            }
            StmtKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.expr(cond, assigned, span);
                let entry = assigned.clone();
                let mut then_assigned = entry.clone();
                let then_falls = self.block(then_branch, &mut then_assigned, span);
                let mut else_assigned = entry.clone();
                let else_falls = match else_branch {
                    Some(else_branch) => self.block(else_branch, &mut else_assigned, span),
                    None => true,
                };
                // Only the paths that reach the join constrain the merge. A
                // branch that returns or breaks cannot carry an unassigned
                // variable to the statements that follow the `if`.
                *assigned = match (then_falls, else_falls) {
                    (true, true) => then_assigned
                        .intersection(&else_assigned)
                        .cloned()
                        .collect(),
                    (true, false) => then_assigned,
                    (false, true) => else_assigned,
                    (false, false) => entry,
                };
                then_falls || else_falls
            }
            StmtKind::While { cond, body } => {
                self.expr(cond, assigned, span);
                let entry = assigned.clone();
                let mut body_assigned = entry.clone();
                let _ = self.block(body, &mut body_assigned, span);
                // The body may run zero times.
                *assigned = entry;
                true
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some(init) = init {
                    self.stmt(init, assigned, span);
                }
                if let Some(cond) = cond {
                    self.expr(cond, assigned, span);
                }
                let entry = assigned.clone();
                let mut body_assigned = entry.clone();
                let _ = self.block(body, &mut body_assigned, span);
                if let Some(step) = step {
                    self.expr(step, &body_assigned, span);
                }
                *assigned = entry;
                true
            }
            StmtKind::DoWhile { body, cond } => {
                let entry = assigned.clone();
                let mut body_assigned = entry.clone();
                let _ = self.block(body, &mut body_assigned, span);
                self.expr(cond, &body_assigned, span);
                *assigned = entry.union(&body_assigned).cloned().collect();
                true
            }
            StmtKind::Switch { scrutinee, cases } => {
                self.expr(scrutinee, assigned, span);
                let entry = assigned.clone();
                let has_default = cases.iter().any(|case| case.label.is_none());
                let mut exit: Option<BTreeSet<String>> = None;
                for case in cases {
                    let mut case_assigned = entry.clone();
                    let falls = self.block(&case.body, &mut case_assigned, span);
                    if !falls {
                        // A case that returns or breaks leaves the switch.
                        continue;
                    }
                    exit = Some(match exit {
                        Some(intersection) => {
                            intersection.intersection(&case_assigned).cloned().collect()
                        }
                        None => case_assigned,
                    });
                }
                if !has_default {
                    exit = Some(match exit {
                        Some(intersection) => intersection.intersection(&entry).cloned().collect(),
                        None => entry.clone(),
                    });
                }
                *assigned = exit.unwrap_or_else(|| entry.clone());
                true
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.expr(value, assigned, span);
                }
                false
            }
            StmtKind::Break | StmtKind::Continue => false,
            StmtKind::Empty => true,
        }
    }

    fn expr(&mut self, expr: &Expr, assigned: &BTreeSet<String>, span: &Span) {
        match &expr.kind {
            ExprKind::Local(name) => {
                if !assigned.contains(name) {
                    self.report(
                        span,
                        format!("local `{name}` is read before it is definitely assigned"),
                    );
                }
            }
            ExprKind::Unary { op, operand, .. } => {
                self.expr(operand, assigned, span);
                let _ = op;
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs, assigned, span);
                self.expr(rhs, assigned, span);
            }
            ExprKind::Assign { op, lhs, rhs } => {
                if op == "=" {
                    self.lhs(lhs, assigned, span, false);
                } else {
                    self.lhs(lhs, assigned, span, true);
                }
                self.expr(rhs, assigned, span);
            }
            ExprKind::Index { base, index } => {
                self.expr(base, assigned, span);
                self.expr(index, assigned, span);
            }
            ExprKind::Member { base, .. } => self.expr(base, assigned, span),
            ExprKind::Cond {
                cond,
                then_branch,
                else_branch,
            } => {
                self.expr(cond, assigned, span);
                self.expr(then_branch, assigned, span);
                self.expr(else_branch, assigned, span);
            }
            ExprKind::InitList(elems) => {
                for elem in elems {
                    self.expr(elem, assigned, span);
                }
            }
            ExprKind::Call { callee, args } => {
                if let recast_ir::Callee::Indirect(callee) = callee {
                    self.expr(callee, assigned, span);
                }
                for arg in args {
                    self.expr(arg, assigned, span);
                }
            }
            ExprKind::Cast { expr, .. } | ExprKind::Paren(expr) => self.expr(expr, assigned, span),
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
        let _ = self.program.types.get(expr.ty);
    }

    fn lhs(&mut self, expr: &Expr, assigned: &BTreeSet<String>, span: &Span, read_target: bool) {
        match &expr.kind {
            ExprKind::Local(name) => {
                if read_target && !assigned.contains(name) {
                    self.report(
                        span,
                        format!("local `{name}` is modified before it is definitely assigned"),
                    );
                }
            }
            ExprKind::Paren(inner) | ExprKind::Cast { expr: inner, .. } => {
                self.lhs(inner, assigned, span, read_target)
            }
            ExprKind::Index { base, index } => {
                self.expr(base, assigned, span);
                self.expr(index, assigned, span);
            }
            ExprKind::Member { base, .. } => self.expr(base, assigned, span),
            _ => self.expr(expr, assigned, span),
        }
    }
}

/// Record the locals an expression statement writes.
fn apply_defs(expr: &Expr, assigned: &mut BTreeSet<String>) {
    match &expr.kind {
        ExprKind::Assign { lhs, .. } => {
            if let Some(name) = crate::cfg::root_local(lhs) {
                assigned.insert(name.to_string());
            }
        }
        ExprKind::Unary { op, operand, .. } if op == "++" || op == "--" => {
            if let Some(name) = crate::cfg::root_local(operand) {
                assigned.insert(name.to_string());
            }
        }
        ExprKind::Binary { lhs, rhs, .. }
        | ExprKind::Index {
            base: lhs,
            index: rhs,
        } => {
            apply_defs(lhs, assigned);
            apply_defs(rhs, assigned);
        }
        ExprKind::Member { base, .. }
        | ExprKind::Cast { expr: base, .. }
        | ExprKind::Paren(base) => apply_defs(base, assigned),
        ExprKind::Cond {
            cond,
            then_branch,
            else_branch,
        } => {
            apply_defs(cond, assigned);
            apply_defs(then_branch, assigned);
            apply_defs(else_branch, assigned);
        }
        ExprKind::InitList(elems) => {
            for elem in elems {
                apply_defs(elem, assigned);
            }
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                apply_defs(arg, assigned);
            }
        }
        _ => {}
    }
}

/// The byte width of an integer type, used by the UB checks.
pub fn int_width(program: &recast_ir::Program, id: TypeId) -> Option<u16> {
    match program.types.canonical(id) {
        Some(Type::Int { bits, .. }) => Some(u16::from(*bits)),
        Some(Type::Char { .. }) => Some(8),
        Some(Type::Bool) => Some(1),
        Some(Type::Enum { name }) => program
            .enum_named(name)
            .and_then(|item| int_width(program, item.underlying)),
        _ => None,
    }
}
