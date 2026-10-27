//! Translation from TIR to Rust.
//!
//! M1 covers the L0 subset at tier T0: scalar and pointer-free functions,
//! structs by value, fixed arrays, enums (as their underlying integer),
//! constants, `if`, `while`, `do-while`, `for`, `switch`, `break` and
//! `continue`.
//!
//! Slice recognition is deliberately conservative: a pointer parameter is
//! treated as a slice only when the next parameter is a `size_t`, and every use
//! of the pointer must be an index and every use of the length must be a
//! standalone local. Anything else is refused rather than translated unsafely.

pub mod rust;

use std::cell::Cell;
use std::collections::BTreeSet;
use std::fmt;

use recast_ir::{
    Block, Callee, Case, Expr, ExprKind, Function, Global, Linkage, Program, RecordDef, Stmt,
    StmtKind, Type, TypeId,
};
use rust::{
    Expr as RExpr, File as RFile, Fn as RFn, Item as RItem, MatchArm, Pattern, Stmt as RStmt,
};

/// Generated Rust source for one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    pub source: String,
}

/// A construct M1 cannot translate without guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslateError {
    pub function: String,
    pub message: String,
}

impl TranslateError {
    fn new(function: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            function: function.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for TranslateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.function, self.message)
    }
}

impl std::error::Error for TranslateError {}

/// Translate every function with a body, plus records and constants.
pub fn translate(program: &Program) -> Result<Translation, TranslateError> {
    let mut file = RFile {
        attrs: vec![
            "#![forbid(unsafe_code)]".to_string(),
            "#![allow(dead_code, non_camel_case_types, non_snake_case, unused_labels)]".to_string(),
        ],
        items: Vec::new(),
        fns: Vec::new(),
    };
    for record in &program.records {
        append_record(program, record, &mut file)?;
    }
    for global in &program.globals {
        append_global(program, global, &mut file)?;
    }
    for function in &program.functions {
        if function.body.is_none() {
            continue;
        }
        let translator = Translator::new(program, function)?;
        file.fns.push(translator.function_item(function)?);
    }
    Ok(Translation {
        source: file.render(),
    })
}

fn append_record(
    program: &Program,
    record: &RecordDef,
    file: &mut RFile,
) -> Result<(), TranslateError> {
    let context = format!("record {}", record.name);
    let mut fields = Vec::with_capacity(record.fields.len());
    for field in &record.fields {
        fields.push((
            field.name.clone(),
            type_name(program, field.ty)
                .map_err(|message| TranslateError::new(&context, message))?,
        ));
    }
    file.items.push(RItem::Struct {
        docs: vec![format!(
            "recast: translated from C struct {} (document 06 section 5.1)",
            record.name
        )],
        name: record.name.clone(),
        fields,
    });
    if record.size > 0 {
        file.items.push(RItem::LayoutAssert {
            name: record.name.clone(),
            size: record.size,
            align: record.align,
        });
    }
    Ok(())
}

fn append_global(
    program: &Program,
    global: &Global,
    file: &mut RFile,
) -> Result<(), TranslateError> {
    if !global.is_const {
        return Err(TranslateError::new(
            &global.name,
            "mutable file-scope variables need the T1 ownership model (document 06 section 14.6); M1 translates constants only",
        ));
    }
    let init = global.init.as_ref().ok_or_else(|| {
        TranslateError::new(
            &global.name,
            "a constant needs an initializer to become a Rust const",
        )
    })?;
    let ty = type_name(program, global.ty)
        .map_err(|message| TranslateError::new(&global.name, message))?;
    let translator = Translator {
        labels: Cell::new(0),
        program,
        function: &global.name,
        ret: global.ty,
        assigned: BTreeSet::new(),
        slices: Vec::new(),
    };
    let value = translator.value(init, global.ty)?;
    file.items.push(RItem::Const {
        name: global.name.clone(),
        ty,
        value,
    });
    Ok(())
}

#[derive(Debug, Clone)]
struct SlicePlan {
    pointer: String,
    length: String,
    element: TypeId,
    is_const: bool,
}

/// Where a statement sits, for `break` and `continue` decisions.
#[derive(Debug, Clone, Default)]
struct Context {
    /// Loops entered since the nearest enclosing `switch` arm.
    break_depth: usize,
    /// Whether a `continue` is legal here.
    can_continue: bool,
    /// The label a `continue` must jump to when it targets a loop whose step
    /// runs after the body.
    continue_label: Option<String>,
    /// The label a `break` must jump to when the loop body is a labeled block,
    /// because an unlabeled `break` inside such a block is an error.
    break_label: Option<String>,
    /// Whether the statement is directly inside a `switch` arm.
    in_arm: bool,
}

impl Context {
    fn loop_body(&self, continue_label: Option<String>, break_label: Option<String>) -> Self {
        Self {
            break_depth: self.break_depth + 1,
            can_continue: true,
            continue_label,
            break_label,
            in_arm: false,
        }
    }

    fn switch_arm(&self) -> Self {
        Self {
            break_depth: 0,
            can_continue: self.can_continue,
            continue_label: self.continue_label.clone(),
            // A `break` inside an arm leaves the switch, not the loop.
            break_label: None,
            in_arm: true,
        }
    }
}

struct Translator<'a> {
    program: &'a Program,
    function: &'a str,
    ret: TypeId,
    assigned: BTreeSet<String>,
    slices: Vec<SlicePlan>,
    /// Serial for generated labels, so two nested loops cannot shadow another
    /// loop's label.
    labels: Cell<u32>,
}

impl<'a> Translator<'a> {
    fn new(program: &'a Program, function: &'a Function) -> Result<Self, TranslateError> {
        let mut slices = Vec::new();
        // Recognize the C idiom `(const T *a, const T *b, ..., size_t n)`:
        // every pointer before a `size_t` shares that length.
        if let Some(index) = function
            .signature
            .params
            .iter()
            .position(|param| is_size_t(program, param.ty))
        {
            let length_name = function.signature.params[index].name.clone();
            let pointers_only = index > 0
                && function.signature.params[..index]
                    .iter()
                    .all(|param| matches!(program.types.get(param.ty), Some(Type::Pointer { .. })));
            if let Some(length_name) = length_name.filter(|_| pointers_only) {
                for pointer in &function.signature.params[..index] {
                    let Some(pointer_name) = &pointer.name else {
                        continue;
                    };
                    let Some(Type::Pointer { pointee, is_const }) = program.types.get(pointer.ty)
                    else {
                        continue;
                    };
                    if !is_slice_element(program, *pointee) {
                        continue;
                    }
                    slices.push(SlicePlan {
                        pointer: pointer_name.clone(),
                        length: length_name.clone(),
                        element: *pointee,
                        is_const: *is_const,
                    });
                }
            }
        }

        let mut assigned = BTreeSet::new();
        if let Some(body) = &function.body {
            collect_assigned(body, &mut assigned);
        }

        Ok(Self {
            program,
            function: &function.name,
            ret: function.signature.ret,
            assigned,
            slices,
            labels: Cell::new(0),
        })
    }

    /// A distinct label name for one generated loop or block.
    fn label(&self, stem: &str) -> String {
        let serial = self.labels.get() + 1;
        self.labels.set(serial);
        format!("{stem}_{serial}")
    }

    fn function_item(&self, function: &Function) -> Result<RFn, TranslateError> {
        let source = function
            .body
            .as_ref()
            .ok_or_else(|| self.error("only function definitions are translated"))?;

        let mut params = Vec::new();
        for param in &function.signature.params {
            let mut name = param
                .name
                .clone()
                .ok_or_else(|| self.error("a parameter has no name"))?;
            if self.slice_by_length(&name).is_some() {
                if self.length_parameter_is_used(source, &name) {
                    return Err(self.error(format!(
                        "slice length {name} is used as a value; M1 only supports `len()`"
                    )));
                }
                continue;
            }
            if let Some(plan) = self.slice_by_pointer(&name) {
                let element = self.rust_type(plan.element)?;
                let reference = if plan.is_const { "&" } else { "&mut " };
                params.push((name, format!("{reference}[{element}]")));
                continue;
            }
            if self.assigned.contains(&name) {
                // C parameters are mutable; Rust parameters are not unless
                // asked, and assigning through a parameter is ordinary L0.
                name = format!("mut {name}");
            }
            params.push((name, self.rust_type(param.ty)?));
        }

        let ret = self.rust_type(function.signature.ret)?;
        let value = !is_unit(self.program, function.signature.ret);
        let body = self.body(source, value)?;
        let span = &function.provenance.span;
        Ok(RFn {
            docs: vec![format!(
                "recast: translated from {}:{}",
                span.file, span.line
            )],
            name: function.name.clone(),
            public: function.linkage == Linkage::External,
            params,
            ret,
            body,
        })
    }

    fn body(&self, block: &Block, returns_value: bool) -> Result<Vec<RStmt>, TranslateError> {
        let ctx = Context::default();
        let mut out = Vec::new();
        for (index, stmt) in block.stmts.iter().enumerate() {
            let last = index + 1 == block.stmts.len();
            if last && returns_value {
                if let StmtKind::Return(Some(value)) = &stmt.kind {
                    out.push(RStmt::Tail(self.value(value, self.ret)?));
                    continue;
                }
            }
            if last && !returns_value {
                if let StmtKind::Return(None) = &stmt.kind {
                    continue;
                }
            }
            out.extend(self.stmt(stmt, &ctx)?);
        }
        if returns_value {
            match block.stmts.last() {
                Some(stmt) if statement_diverges(stmt) => {}
                _ => {
                    // Falling off the end of a non-void C function is UB if the
                    // value is used. A panic preserves that reading instead of
                    // inventing a value.
                    out.push(RStmt::Expr(RExpr::Call {
                        callee: Box::new(RExpr::Path("unreachable!".to_string())),
                        args: Vec::new(),
                    }));
                }
            }
        }
        Ok(out)
    }

    fn nested(&self, block: &Block, ctx: &Context) -> Result<Vec<RStmt>, TranslateError> {
        self.nested_stmts(&block.stmts, ctx)
    }

    fn nested_stmts(&self, stmts: &[Stmt], ctx: &Context) -> Result<Vec<RStmt>, TranslateError> {
        let mut out = Vec::new();
        for stmt in stmts {
            out.extend(self.stmt(stmt, ctx)?);
        }
        Ok(out)
    }

    fn stmt(&self, stmt: &Stmt, ctx: &Context) -> Result<Vec<RStmt>, TranslateError> {
        let single = match &stmt.kind {
            StmtKind::Decl { name, ty, init } => vec![self.decl(name, *ty, init.as_ref())?],
            StmtKind::Expr(expr) => vec![self.expr_statement(expr)?],
            StmtKind::If {
                cond,
                then_branch,
                else_branch,
            } => vec![RStmt::If {
                cond: self.condition(cond)?,
                then_body: self.nested(then_branch, ctx)?,
                else_body: match else_branch {
                    Some(block) => Some(self.nested(block, ctx)?),
                    None => None,
                },
            }],
            StmtKind::While { cond, body } => {
                let body_ctx = ctx.loop_body(None, None);
                vec![RStmt::While {
                    cond: self.condition(cond)?,
                    body: self.nested(body, &body_ctx)?,
                }]
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => self.for_loop(init.as_deref(), cond.as_ref(), step.as_ref(), body, ctx)?,
            StmtKind::DoWhile { body, cond } => self.do_while(body, cond, ctx)?,
            StmtKind::Switch { scrutinee, cases } => {
                vec![self.switch(scrutinee, cases, ctx)?]
            }
            StmtKind::Return(value) => vec![RStmt::Return(match value {
                Some(value) => Some(self.value(value, self.ret)?),
                None => None,
            })],
            StmtKind::Break => {
                if ctx.in_arm && ctx.break_depth == 0 {
                    return Err(self.error(
                        "a break in the middle of a switch case needs a labeled block (document 06 section 10.2); M1 supports only a trailing break",
                    ));
                }
                match &ctx.break_label {
                    Some(label) => vec![RStmt::BreakLabel(label.clone())],
                    None => vec![RStmt::Break],
                }
            }
            StmtKind::Continue => {
                if !ctx.can_continue {
                    return Err(self.error("continue has no enclosing loop"));
                }
                match &ctx.continue_label {
                    Some(label) => vec![RStmt::BreakLabel(label.clone())],
                    None => vec![RStmt::Continue],
                }
            }
            StmtKind::Empty => Vec::new(),
        };
        Ok(single)
    }

    fn for_loop(
        &self,
        init: Option<&Stmt>,
        cond: Option<&Expr>,
        step: Option<&Expr>,
        body: &Block,
        ctx: &Context,
    ) -> Result<Vec<RStmt>, TranslateError> {
        let mut out = Vec::new();
        if let Some(init) = init {
            out.extend(self.stmt(init, ctx)?);
        }
        let condition = match cond {
            Some(cond) => self.condition(cond)?,
            None => RExpr::Bool(true),
        };
        if block_has_continue(body) {
            // `continue` must still run the step, so the body is a labeled
            // block that `continue` leaves and the step runs after it. A
            // `break` inside that block cannot leave the loop unlabeled, so
            // the loop carries a label of its own.
            let body_label = self.label("for_body");
            let loop_label = self.label("for_loop");
            let body_ctx = ctx.loop_body(Some(body_label.clone()), Some(loop_label.clone()));
            let mut inner = vec![RStmt::If {
                cond: RExpr::Unary {
                    op: "!",
                    expr: Box::new(condition),
                },
                then_body: vec![RStmt::BreakLabel(loop_label.clone())],
                else_body: None,
            }];
            inner.push(RStmt::Block {
                label: Some(body_label),
                body: self.nested(body, &body_ctx)?,
            });
            if let Some(step) = step {
                inner.push(self.expr_statement(step)?);
            }
            out.push(RStmt::Loop {
                label: Some(loop_label),
                body: inner,
            });
        } else {
            let body_ctx = ctx.loop_body(None, None);
            let mut body_stmts = self.nested(body, &body_ctx)?;
            if let Some(step) = step {
                body_stmts.push(self.expr_statement(step)?);
            }
            out.push(RStmt::While {
                cond: condition,
                body: body_stmts,
            });
        }
        Ok(out)
    }

    fn do_while(
        &self,
        body: &Block,
        cond: &Expr,
        ctx: &Context,
    ) -> Result<Vec<RStmt>, TranslateError> {
        let condition = self.condition(cond)?;
        let mut inner = Vec::new();
        let mut loop_label = None;
        if block_has_continue(body) {
            let body_label = self.label("do_body");
            let label = self.label("do_loop");
            let body_ctx = ctx.loop_body(Some(body_label.clone()), Some(label.clone()));
            inner.push(RStmt::Block {
                label: Some(body_label),
                body: self.nested(body, &body_ctx)?,
            });
            loop_label = Some(label);
        } else {
            let body_ctx = ctx.loop_body(None, None);
            inner.extend(self.nested(body, &body_ctx)?);
        }
        let finish = RStmt::If {
            cond: RExpr::Unary {
                op: "!",
                expr: Box::new(condition),
            },
            then_body: vec![match &loop_label {
                Some(label) => RStmt::BreakLabel(label.clone()),
                None => RStmt::Break,
            }],
            else_body: None,
        };
        inner.push(finish);
        Ok(vec![RStmt::Loop {
            label: loop_label,
            body: inner,
        }])
    }

    fn switch(
        &self,
        scrutinee: &Expr,
        cases: &[Case],
        ctx: &Context,
    ) -> Result<RStmt, TranslateError> {
        let scrutinee = self.value(scrutinee, scrutinee.ty)?;
        let arm_ctx = ctx.switch_arm();
        let mut arms: Vec<MatchArm> = Vec::new();
        let mut pending: Vec<Pattern> = Vec::new();
        let mut pending_default = false;
        let mut has_default = false;

        for case in cases {
            match case.label {
                Some(label) => pending.push(Pattern::Int(label)),
                None => pending_default = true,
            }
            if case.falls_through {
                if case.body.stmts.is_empty() {
                    // An empty case just adds another label to the next arm.
                    continue;
                }
                return Err(self.error(
                    "switch fallthrough with statements is outside the M1 subset (document 06 section 10.2)",
                ));
            }

            let mut stmts: &[Stmt] = &case.body.stmts;
            if let Some(Stmt {
                kind: StmtKind::Break,
                ..
            }) = stmts.last()
            {
                stmts = &stmts[..stmts.len() - 1];
            }
            let body = self.nested_stmts(stmts, &arm_ctx)?;
            let patterns = if pending_default {
                has_default = true;
                vec![Pattern::Default]
            } else {
                std::mem::take(&mut pending)
            };
            arms.push(MatchArm { patterns, body });
            pending_default = false;
        }

        if pending_default && !has_default {
            arms.push(MatchArm {
                patterns: vec![Pattern::Default],
                body: Vec::new(),
            });
            has_default = true;
        }
        if !pending.is_empty() {
            return Err(self.error("a switch case has no body"));
        }
        if !has_default {
            arms.push(MatchArm {
                patterns: vec![Pattern::Default],
                body: Vec::new(),
            });
        }
        // `_` matches everything, so a `default` that is not last in C would
        // make every later case dead in Rust. Each arm's body is complete
        // (M1 refuses fallthrough with statements), so moving the `default`
        // arm to the end keeps C's meaning.
        if let Some(index) = arms
            .iter()
            .position(|arm| arm.patterns.contains(&Pattern::Default))
        {
            if index + 1 != arms.len() {
                let default_arm = arms.remove(index);
                arms.push(default_arm);
            }
        }
        Ok(RStmt::Match { scrutinee, arms })
    }

    fn decl(&self, name: &str, ty: TypeId, init: Option<&Expr>) -> Result<RStmt, TranslateError> {
        let rust_ty = self.rust_type(ty)?;
        let init = match init {
            Some(init) => Some(self.value(init, ty)?),
            None => match self.program.types.get(ty) {
                Some(Type::Array { .. }) | Some(Type::Record { .. }) => {
                    // C leaves these indeterminate. Zeroing keeps the generated
                    // Rust total; definite-assignment analysis reports reads
                    // that depend on it.
                    Some(zero_value(self.program, ty).map_err(|message| self.error(message))?)
                }
                _ => None,
            },
        };
        Ok(RStmt::Let {
            name: name.to_string(),
            mutable: self.assigned.contains(name),
            ty: rust_ty,
            init,
        })
    }

    fn expr_statement(&self, expr: &Expr) -> Result<RStmt, TranslateError> {
        match &expr.kind {
            ExprKind::Assign { op, lhs, rhs } => self.assign_statement(op, lhs, rhs),
            ExprKind::Unary {
                op,
                operand,
                postfix: _,
            } if op == "++" || op == "--" => {
                let target = self.expr(operand)?;
                let step = if op == "++" {
                    "wrapping_add"
                } else {
                    "wrapping_sub"
                };
                if is_unsigned(self.program, operand.ty) {
                    Ok(RStmt::Assign {
                        target: self.expr(operand)?,
                        op: "=",
                        value: RExpr::Method {
                            receiver: Box::new(target),
                            method: step.to_string(),
                            args: vec![RExpr::int(1)],
                        },
                    })
                } else {
                    Ok(RStmt::Assign {
                        target,
                        op: if op == "++" { "+=" } else { "-=" },
                        value: RExpr::int(1),
                    })
                }
            }
            _ => Ok(RStmt::Expr(self.expr(expr)?)),
        }
    }

    fn assign_statement(&self, op: &str, lhs: &Expr, rhs: &Expr) -> Result<RStmt, TranslateError> {
        let op = match op {
            "=" => "=",
            "+=" => "+=",
            "-=" => "-=",
            "*=" => "*=",
            "/=" => "/=",
            "%=" => "%=",
            "&=" => "&=",
            "|=" => "|=",
            "^=" => "^=",
            other => {
                return Err(self.error(format!("assignment operator {other} is not supported")))
            }
        };
        let wrapping = if is_unsigned(self.program, lhs.ty) {
            match op {
                "+=" => Some("wrapping_add"),
                "-=" => Some("wrapping_sub"),
                "*=" => Some("wrapping_mul"),
                _ => None,
            }
        } else {
            None
        };
        if let Some(method) = wrapping {
            let base = self.expr(lhs)?;
            return Ok(RStmt::Assign {
                target: base.clone(),
                op: "=",
                value: RExpr::Method {
                    receiver: Box::new(base),
                    method: method.to_string(),
                    args: vec![self.value(rhs, lhs.ty)?],
                },
            });
        }
        Ok(RStmt::Assign {
            target: self.expr(lhs)?,
            op,
            value: self.value(rhs, lhs.ty)?,
        })
    }

    fn expr(&self, expr: &Expr) -> Result<RExpr, TranslateError> {
        match &expr.kind {
            ExprKind::IntLit(value) => Ok(self.int_literal(*value, expr.ty)),
            ExprKind::FloatLit(value) => Ok(RExpr::Float(*value)),
            ExprKind::BoolLit(value) => Ok(RExpr::Bool(*value)),
            ExprKind::CharLit(value) => Ok(self.int_literal(i64::from(*value), expr.ty)),
            ExprKind::StringLit(value) => Ok(RExpr::Bytes(value.clone().into_bytes())),
            ExprKind::Local(name) => {
                if let Some(plan) = self.slice_by_length(name) {
                    return Ok(RExpr::Method {
                        receiver: Box::new(RExpr::Path(plan.pointer.clone())),
                        method: "len".to_string(),
                        args: Vec::new(),
                    });
                }
                if self.slice_by_pointer(name).is_some() {
                    return Ok(RExpr::Path(name.clone()));
                }
                Ok(RExpr::Path(name.clone()))
            }
            ExprKind::Global(id) => {
                let global = self
                    .program
                    .global(id)
                    .ok_or_else(|| self.error(format!("global {id} does not exist")))?;
                if !global.is_const {
                    return Err(self.error(format!(
                        "mutable global {} needs the T1 ownership model",
                        global.name
                    )));
                }
                Ok(RExpr::Path(global.name.clone()))
            }
            ExprKind::EnumConst { value, .. } => Ok(self.int_literal(*value, expr.ty)),
            ExprKind::Unary { op, operand, .. } => match op.as_str() {
                "-" if is_unsigned(self.program, operand.ty) => Ok(RExpr::Method {
                    receiver: Box::new(self.typed(self.value(operand, operand.ty)?, operand.ty)),
                    method: "wrapping_neg".to_string(),
                    args: Vec::new(),
                }),
                // A negated literal keeps its own type: without it, a cast of
                // the result would infer the target into the literal and reject
                // the negation (`(-1) as u32` is an error, `-1i32 as u32` is not).
                "-" => Ok(RExpr::Unary {
                    op: "-",
                    expr: Box::new(self.typed(self.value(operand, operand.ty)?, operand.ty)),
                }),
                "~" => Ok(RExpr::Unary {
                    op: "!",
                    expr: Box::new(self.typed(self.value(operand, operand.ty)?, operand.ty)),
                }),
                "+" => self.value(operand, operand.ty),
                "!" => Ok(RExpr::Unary {
                    op: "!",
                    expr: Box::new(self.condition(operand)?),
                }),
                "++" | "--" => Err(self.error(format!("{op} is only translated as a statement"))),
                other => Err(self.error(format!("unary operator {other} is not supported"))),
            },
            ExprKind::Binary { op, lhs, rhs } => self.binary(op, lhs, rhs),
            ExprKind::Assign { .. } => {
                Err(self.error("an assignment used as a value is not supported"))
            }
            ExprKind::Index { base, index } => {
                let base = self.index_base(base)?;
                let lowered = self.value(index, index.ty)?;
                let index_type = self.rust_type(index.ty)?;
                let index = if index_type == "usize" {
                    lowered
                } else {
                    RExpr::Cast {
                        expr: Box::new(lowered),
                        ty: "usize".to_string(),
                    }
                };
                Ok(RExpr::Index {
                    base: Box::new(base),
                    index: Box::new(index),
                })
            }
            ExprKind::Member { base, field, arrow } => {
                if *arrow {
                    return Err(self.error(
                        "pointer member access is outside the M1 subset; use a slice or a value",
                    ));
                }
                Ok(RExpr::Field {
                    base: Box::new(self.expr(base)?),
                    field: field.clone(),
                })
            }
            ExprKind::Cond {
                cond,
                then_branch,
                else_branch,
            } => Ok(RExpr::If {
                cond: Box::new(self.condition(cond)?),
                then_branch: Box::new(self.value(then_branch, expr.ty)?),
                else_branch: Box::new(self.value(else_branch, expr.ty)?),
            }),
            ExprKind::InitList(_) => self.aggregate(expr, expr.ty),
            ExprKind::Call { callee, args } => {
                let (name, parameters) = match callee {
                    Callee::Direct(id) => {
                        let function = self.program.function(id).ok_or_else(|| {
                            self.error(format!("call target {id} does not exist"))
                        })?;
                        (
                            function.name.clone(),
                            function
                                .signature
                                .params
                                .iter()
                                .map(|param| param.ty)
                                .collect::<Vec<_>>(),
                        )
                    }
                    Callee::Indirect(_) => {
                        return Err(self.error("indirect calls are not supported"))
                    }
                    Callee::Unresolved(name) => {
                        return Err(self.error(format!("call to unresolved function {name}")))
                    }
                };
                let mut lowered = Vec::with_capacity(args.len());
                for (index, arg) in args.iter().enumerate() {
                    let expected = parameters.get(index).copied().unwrap_or(arg.ty);
                    lowered.push(self.value(arg, expected)?);
                }
                Ok(RExpr::Call {
                    callee: Box::new(RExpr::Path(name)),
                    args: lowered,
                })
            }
            ExprKind::Cast { to, expr, .. } => self.cast(*to, expr),
            // Parentheses are re-derived by the printer from precedence, so
            // the source grouping node adds nothing here.
            ExprKind::Paren(expr) => self.expr(expr),
            ExprKind::Null | ExprKind::Unsupported(_) => Err(self.error(format!(
                "expression {:?} is outside the M1 translation subset",
                expr.kind
            ))),
        }
    }

    fn aggregate(&self, expr: &Expr, ty: TypeId) -> Result<RExpr, TranslateError> {
        let ExprKind::InitList(elems) = &expr.kind else {
            return Err(self.error("expected an initializer list"));
        };
        match self.program.types.get(ty) {
            Some(Type::Record { name }) => {
                let record = self
                    .program
                    .record(name)
                    .ok_or_else(|| self.error(format!("record {name} is not defined")))?;
                let mut fields = Vec::with_capacity(record.fields.len());
                for (index, field) in record.fields.iter().enumerate() {
                    let value = elems.get(index).ok_or_else(|| {
                        self.error(format!("record {name} is missing field {}", field.name))
                    })?;
                    fields.push((field.name.clone(), self.value(value, field.ty)?));
                }
                Ok(RExpr::StructLit {
                    path: name.clone(),
                    fields,
                })
            }
            Some(Type::Array { elem, .. }) => {
                let mut values = Vec::with_capacity(elems.len());
                for elem_expr in elems {
                    values.push(self.value(elem_expr, *elem)?);
                }
                Ok(RExpr::ArrayLit(values))
            }
            _ => Err(self.error("an initializer list needs a record or array type")),
        }
    }

    fn binary(&self, op: &str, lhs: &Expr, rhs: &Expr) -> Result<RExpr, TranslateError> {
        match op {
            "&&" | "||" => Ok(RExpr::Binary {
                op: if op == "&&" { "&&" } else { "||" },
                lhs: Box::new(self.condition(lhs)?),
                rhs: Box::new(self.condition(rhs)?),
            }),
            "==" | "!=" | "<" | "<=" | ">" | ">=" => Ok(RExpr::Binary {
                op: match op {
                    "==" => "==",
                    "!=" => "!=",
                    "<" => "<",
                    "<=" => "<=",
                    ">" => ">",
                    _ => ">=",
                },
                lhs: Box::new(self.value(lhs, lhs.ty)?),
                rhs: Box::new(self.value(rhs, rhs.ty)?),
            }),
            "<<" | ">>" => self.shift(op, lhs, rhs),
            "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" => {
                let lhs_expr = self.value(lhs, lhs.ty)?;
                let rhs_expr = self.value(rhs, rhs.ty)?;
                if matches!(op, "+" | "-" | "*") && is_unsigned(self.program, lhs.ty) {
                    let method = match op {
                        "+" => "wrapping_add",
                        "-" => "wrapping_sub",
                        _ => "wrapping_mul",
                    };
                    return Ok(RExpr::Method {
                        receiver: Box::new(self.typed(lhs_expr, lhs.ty)),
                        method: method.to_string(),
                        args: vec![rhs_expr],
                    });
                }
                Ok(RExpr::Binary {
                    op: match op {
                        "+" => "+",
                        "-" => "-",
                        "*" => "*",
                        "/" => "/",
                        "%" => "%",
                        "&" => "&",
                        "|" => "|",
                        _ => "^",
                    },
                    lhs: Box::new(lhs_expr),
                    rhs: Box::new(rhs_expr),
                })
            }
            other => Err(self.error(format!("binary operator {other} is not supported"))),
        }
    }

    fn shift(&self, op: &str, lhs: &Expr, rhs: &Expr) -> Result<RExpr, TranslateError> {
        let lhs_expr = self.value(lhs, lhs.ty)?;
        let rhs_expr = self.value(rhs, rhs.ty)?;
        let width = self.int_width(lhs.ty)?;
        if let ExprKind::IntLit(count) = &rhs.kind {
            if *count >= 0 && (*count as u64) < width {
                return Ok(RExpr::Binary {
                    op: if op == "<<" { "<<" } else { ">>" },
                    lhs: Box::new(lhs_expr),
                    rhs: Box::new(rhs_expr),
                });
            }
        }
        let method = if op == "<<" {
            "checked_shl"
        } else {
            "checked_shr"
        };
        let checked = RExpr::Method {
            receiver: Box::new(self.typed(lhs_expr, lhs.ty)),
            method: method.to_string(),
            args: vec![RExpr::Cast {
                expr: Box::new(rhs_expr),
                ty: "u32".to_string(),
            }],
        };
        Ok(RExpr::Method {
            receiver: Box::new(checked),
            method: "expect".to_string(),
            args: vec![RExpr::Str("shift count out of range (UB-007)".to_string())],
        })
    }

    fn condition(&self, expr: &Expr) -> Result<RExpr, TranslateError> {
        if let ExprKind::Cast {
            to, expr: inner, ..
        } = &expr.kind
        {
            if is_bool(self.program, *to) {
                return self.condition(inner);
            }
        }
        match &expr.kind {
            ExprKind::Binary { op, lhs, rhs }
                if matches!(
                    op.as_str(),
                    "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||"
                ) =>
            {
                self.binary(op, lhs, rhs)
            }
            ExprKind::Paren(inner) => self.condition(inner),
            ExprKind::Unary { op, operand, .. } if op == "!" => Ok(RExpr::Unary {
                op: "!",
                expr: Box::new(self.condition(operand)?),
            }),
            ExprKind::BoolLit(value) => Ok(RExpr::Bool(*value)),
            ExprKind::IntLit(value) => Ok(RExpr::Bool(*value != 0)),
            _ if is_bool(self.program, expr.ty) => self.expr(expr),
            _ => Ok(RExpr::Binary {
                op: "!=",
                lhs: Box::new(self.expr(expr)?),
                rhs: Box::new(RExpr::int(0)),
            }),
        }
    }

    /// Translate `expr` in a position that needs `expected`, inserting the
    /// `as` cast C relies on when a comparison result is used as an integer.
    fn value(&self, expr: &Expr, expected: TypeId) -> Result<RExpr, TranslateError> {
        let rendered = self.expr(expr)?;
        if self.is_boolean_expression(expr) && !is_bool(self.program, expected) {
            let ty = self.rust_type(expected)?;
            return Ok(RExpr::Cast {
                expr: Box::new(rendered),
                ty,
            });
        }
        Ok(rendered)
    }

    /// Whether the expression renders as a Rust `bool`, which can differ from
    /// its C type: comparisons have type `int` in C but produce `bool` here.
    fn is_boolean_expression(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::BoolLit(_) => true,
            ExprKind::Binary { op, .. } => matches!(
                op.as_str(),
                "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||"
            ),
            ExprKind::Unary { op, .. } => op == "!",
            ExprKind::Cast { to, .. } => is_bool(self.program, *to),
            ExprKind::Paren(inner) => self.is_boolean_expression(inner),
            ExprKind::Cond {
                then_branch,
                else_branch,
                ..
            } => self.is_boolean_expression(then_branch) && self.is_boolean_expression(else_branch),
            _ => is_bool(self.program, expr.ty),
        }
    }

    /// An integer literal, spelled with its Rust type when Rust could not infer
    /// the C type from context: a value outside `i32` (Rust's default) would
    /// either not compile or change value.
    fn int_literal(&self, value: i64, ty: TypeId) -> RExpr {
        let out_of_range = value > i64::from(i32::MAX) || value < i64::from(i32::MIN);
        // An unsigned value stored as a negative bit pattern (a 64-bit constant
        // above `i64::MAX`) needs its type as well, so it prints unsigned.
        let negative_unsigned = value < 0 && is_unsigned(self.program, ty);
        match self.rust_type(ty) {
            Ok(rust) if out_of_range || negative_unsigned => RExpr::typed_int(value, rust),
            _ => RExpr::int(value),
        }
    }

    /// An expression that Rust cannot type on its own: the receiver of a
    /// method call (E0689) or the operand of a unary operator, where the
    /// surrounding type would otherwise be inferred into the literal.
    fn typed(&self, expr: RExpr, ty: TypeId) -> RExpr {
        match (expr, self.rust_type(ty)) {
            (RExpr::Int { value, suffix: None }, Ok(rust)) => RExpr::typed_int(value, rust),
            (expr, _) => expr,
        }
    }

    fn cast(&self, to: TypeId, expr: &Expr) -> Result<RExpr, TranslateError> {
        if is_bool(self.program, to) {
            return self.condition(expr);
        }
        if same_type(self.program, to, expr.ty) {
            if self.is_boolean_expression(expr) && !is_bool(self.program, to) {
                let target = self.rust_type(to)?;
                return Ok(RExpr::Cast {
                    expr: Box::new(self.expr(expr)?),
                    ty: target,
                });
            }
            return self.expr(expr);
        }
        if let ExprKind::IntLit(value) = &expr.kind {
            // Rust infers the target width from the binding or parameter, and
            // an unsuffixed literal keeps the output free of redundant casts.
            return Ok(RExpr::int(*value));
        }
        if matches!(
            self.program.types.get(to),
            Some(Type::Record { .. }) | Some(Type::Array { .. })
        ) {
            return Err(self.error("C cannot cast to an aggregate type"));
        }
        let target = self.rust_type(to)?;
        Ok(RExpr::Cast {
            expr: Box::new(self.expr(expr)?),
            ty: target,
        })
    }

    fn index_base(&self, base: &Expr) -> Result<RExpr, TranslateError> {
        let mut current = base;
        loop {
            match &current.kind {
                ExprKind::Paren(inner) | ExprKind::Cast { expr: inner, .. } => current = inner,
                ExprKind::Local(name) => {
                    if self.slice_by_pointer(name).is_some() {
                        return Ok(RExpr::Path(name.clone()));
                    }
                    return match self.program.types.get(current.ty) {
                        Some(Type::Array { .. }) => Ok(RExpr::Path(name.clone())),
                        _ => Err(self.error(format!(
                            "indexing {name} is only supported for arrays and slice parameters"
                        ))),
                    };
                }
                _ => return Err(self.error("unsupported index base")),
            }
        }
    }

    fn rust_type(&self, id: TypeId) -> Result<String, TranslateError> {
        match self.program.types.get(id) {
            Some(Type::Pointer { .. }) => Err(self.error(
                "a pointer type is only translated as a slice with a following size_t parameter",
            )),
            _ => type_name(self.program, id).map_err(|message| self.error(message)),
        }
    }

    fn int_width(&self, id: TypeId) -> Result<u64, TranslateError> {
        match self.program.types.canonical(id) {
            Some(Type::Int { bits, .. }) => Ok(u64::from(*bits)),
            Some(Type::Char { .. }) => Ok(8),
            Some(Type::Bool) => Ok(1),
            Some(Type::Enum { name }) => {
                let item = self
                    .program
                    .enum_named(name)
                    .ok_or_else(|| self.error(format!("enum {name} is not defined")))?;
                self.int_width(item.underlying)
            }
            _ => Err(self.error("shift operands must be integers")),
        }
    }

    fn slice_by_pointer(&self, name: &str) -> Option<&SlicePlan> {
        self.slices.iter().find(|plan| plan.pointer == name)
    }

    fn slice_by_length(&self, name: &str) -> Option<&SlicePlan> {
        self.slices.iter().find(|plan| plan.length == name)
    }

    fn length_parameter_is_used(&self, block: &Block, name: &str) -> bool {
        !uses_local(block, name)
    }

    fn error(&self, message: impl Into<String>) -> TranslateError {
        TranslateError::new(self.function, message)
    }
}

fn type_name(program: &Program, id: TypeId) -> Result<String, String> {
    match program.types.get(id) {
        Some(Type::Void) => Ok("()".to_string()),
        Some(Type::Bool) => Ok("bool".to_string()),
        Some(Type::Char { signed }) => Ok(if *signed { "i8" } else { "u8" }.to_string()),
        Some(Type::Int { bits, signed, .. }) => {
            Ok(format!("{}{}", if *signed { 'i' } else { 'u' }, bits))
        }
        Some(Type::Float { bits, .. }) => Ok(format!("f{bits}")),
        Some(Type::Array { elem, len }) => Ok(format!("[{}; {len}]", type_name(program, *elem)?)),
        Some(Type::Record { name }) => Ok(name.clone()),
        Some(Type::Enum { name }) => {
            let item = program
                .enum_named(name)
                .ok_or_else(|| format!("enum {name} is not defined"))?;
            type_name(program, item.underlying)
        }
        Some(Type::Named { name, canonical }) => {
            if name == "size_t" {
                Ok("usize".to_string())
            } else if name == "ssize_t" || name == "ptrdiff_t" {
                Ok("isize".to_string())
            } else {
                type_name(program, *canonical)
            }
        }
        Some(Type::Pointer { .. }) => Err(
            "a pointer type is only translated as a slice with a following size_t parameter"
                .to_string(),
        ),
        Some(Type::Unknown { spelling }) => Err(format!("unknown type {spelling}")),
        None => Err(format!("missing type {id}")),
    }
}

fn zero_value(program: &Program, id: TypeId) -> Result<RExpr, String> {
    match program.types.get(id) {
        Some(Type::Bool) => Ok(RExpr::Bool(false)),
        Some(Type::Char { .. }) | Some(Type::Int { .. }) | Some(Type::Enum { .. }) => {
            Ok(RExpr::int(0))
        }
        Some(Type::Float { .. }) => Ok(RExpr::Float(0.0)),
        Some(Type::Array { elem, len }) => Ok(RExpr::Repeat {
            value: Box::new(zero_value(program, *elem)?),
            len: *len,
        }),
        Some(Type::Record { name }) => {
            let record = program
                .record(name)
                .ok_or_else(|| format!("record {name} is not defined"))?;
            let mut fields = Vec::with_capacity(record.fields.len());
            for field in &record.fields {
                fields.push((field.name.clone(), zero_value(program, field.ty)?));
            }
            Ok(RExpr::StructLit {
                path: name.clone(),
                fields,
            })
        }
        Some(Type::Named { canonical, .. }) => zero_value(program, *canonical),
        other => Err(format!("cannot zero-initialize {other:?}")),
    }
}

fn collect_assigned(block: &Block, names: &mut BTreeSet<String>) {
    for stmt in &block.stmts {
        collect_stmt_assignments(stmt, names);
    }
}

fn collect_stmt_assignments(stmt: &Stmt, names: &mut BTreeSet<String>) {
    match &stmt.kind {
        StmtKind::Decl { init, .. } => {
            if let Some(init) = init {
                collect_expr_assignments(init, names);
            }
        }
        StmtKind::Expr(expr) => collect_expr_assignments(expr, names),
        StmtKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_expr_assignments(cond, names);
            collect_assigned(then_branch, names);
            if let Some(else_branch) = else_branch {
                collect_assigned(else_branch, names);
            }
        }
        StmtKind::While { cond, body } => {
            collect_expr_assignments(cond, names);
            collect_assigned(body, names);
        }
        StmtKind::For {
            init,
            cond,
            step,
            body,
        } => {
            if let Some(init) = init {
                collect_stmt_assignments(init, names);
            }
            if let Some(cond) = cond {
                collect_expr_assignments(cond, names);
            }
            if let Some(step) = step {
                collect_expr_assignments(step, names);
            }
            collect_assigned(body, names);
        }
        StmtKind::DoWhile { body, cond } => {
            collect_assigned(body, names);
            collect_expr_assignments(cond, names);
        }
        StmtKind::Switch { scrutinee, cases } => {
            collect_expr_assignments(scrutinee, names);
            for case in cases {
                collect_assigned(&case.body, names);
            }
        }
        StmtKind::Return(Some(value)) => collect_expr_assignments(value, names),
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Empty => {}
    }
}

fn collect_expr_assignments(expr: &Expr, names: &mut BTreeSet<String>) {
    match &expr.kind {
        ExprKind::Assign { lhs, rhs, .. } => {
            if let Some(name) = root_local(lhs) {
                names.insert(name.to_string());
            }
            collect_expr_assignments(lhs, names);
            collect_expr_assignments(rhs, names);
        }
        ExprKind::Unary { op, operand, .. } => {
            if op == "++" || op == "--" {
                if let Some(name) = root_local(operand) {
                    names.insert(name.to_string());
                }
            }
            collect_expr_assignments(operand, names);
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            collect_expr_assignments(lhs, names);
            collect_expr_assignments(rhs, names);
        }
        ExprKind::Index { base, index } => {
            collect_expr_assignments(base, names);
            collect_expr_assignments(index, names);
        }
        ExprKind::Member { base, .. } => collect_expr_assignments(base, names),
        ExprKind::Cond {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_expr_assignments(cond, names);
            collect_expr_assignments(then_branch, names);
            collect_expr_assignments(else_branch, names);
        }
        ExprKind::InitList(elems) => {
            for elem in elems {
                collect_expr_assignments(elem, names);
            }
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                collect_expr_assignments(arg, names);
            }
        }
        ExprKind::Cast { expr, .. } | ExprKind::Paren(expr) => {
            collect_expr_assignments(expr, names);
        }
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

/// The local whose storage an assignment writes through fields and indexes.
fn root_local(expr: &Expr) -> Option<&str> {
    match &expr.kind {
        ExprKind::Local(name) => Some(name),
        ExprKind::Paren(inner) | ExprKind::Cast { expr: inner, .. } => root_local(inner),
        ExprKind::Member { base, .. } => root_local(base),
        ExprKind::Index { base, .. } => root_local(base),
        _ => None,
    }
}

/// Whether control cannot reach the end of this statement.
fn statement_diverges(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) => true,
        StmtKind::If {
            then_branch,
            else_branch: Some(else_branch),
            ..
        } => block_diverges(then_branch) && block_diverges(else_branch),
        StmtKind::Switch { cases, .. } => {
            cases.iter().any(|case| case.label.is_none())
                && cases
                    .iter()
                    // An empty case that falls through adds a label to the
                    // next arm; it has no body of its own to diverge.
                    .filter(|case| !(case.falls_through && case.body.stmts.is_empty()))
                    .all(|case| block_diverges(&case.body))
        }
        _ => false,
    }
}

fn block_diverges(block: &Block) -> bool {
    match block.stmts.last() {
        Some(stmt) => statement_diverges(stmt),
        None => false,
    }
}

fn block_has_continue(block: &Block) -> bool {
    block.stmts.iter().any(stmt_has_continue)
}

/// Whether the block contains a `continue` that targets its own loop, rather
/// than a loop nested inside it.
fn stmt_has_continue(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Continue => true,
        StmtKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            block_has_continue(then_branch) || else_branch.as_ref().is_some_and(block_has_continue)
        }
        StmtKind::Switch { cases, .. } => cases.iter().any(|case| block_has_continue(&case.body)),
        // A continue inside a nested loop belongs to that loop.
        StmtKind::While { .. }
        | StmtKind::For { .. }
        | StmtKind::DoWhile { .. }
        | StmtKind::Decl { .. }
        | StmtKind::Expr(_)
        | StmtKind::Return(_)
        | StmtKind::Break
        | StmtKind::Empty => false,
    }
}

fn uses_local(block: &Block, name: &str) -> bool {
    block.stmts.iter().any(|stmt| stmt_uses_local(stmt, name))
}

fn stmt_uses_local(stmt: &Stmt, name: &str) -> bool {
    match &stmt.kind {
        StmtKind::Decl { init, .. } => init.as_ref().is_some_and(|e| expr_uses_local(e, name)),
        StmtKind::Expr(expr) => expr_uses_local(expr, name),
        StmtKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            expr_uses_local(cond, name)
                || uses_local(then_branch, name)
                || else_branch.as_ref().is_some_and(|b| uses_local(b, name))
        }
        StmtKind::While { cond, body } => expr_uses_local(cond, name) || uses_local(body, name),
        StmtKind::For {
            init,
            cond,
            step,
            body,
        } => {
            init.as_ref().is_some_and(|s| stmt_uses_local(s, name))
                || cond.as_ref().is_some_and(|e| expr_uses_local(e, name))
                || step.as_ref().is_some_and(|e| expr_uses_local(e, name))
                || uses_local(body, name)
        }
        StmtKind::DoWhile { body, cond } => uses_local(body, name) || expr_uses_local(cond, name),
        StmtKind::Switch { scrutinee, cases } => {
            expr_uses_local(scrutinee, name)
                || cases.iter().any(|case| uses_local(&case.body, name))
        }
        StmtKind::Return(Some(value)) => expr_uses_local(value, name),
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Empty => false,
    }
}

fn expr_uses_local(expr: &Expr, name: &str) -> bool {
    match &expr.kind {
        ExprKind::Local(other) => other == name,
        ExprKind::Unary { operand, .. } => expr_uses_local(operand, name),
        ExprKind::Binary { lhs, rhs, .. }
        | ExprKind::Assign { lhs, rhs, .. }
        | ExprKind::Index {
            base: lhs,
            index: rhs,
        } => expr_uses_local(lhs, name) || expr_uses_local(rhs, name),
        ExprKind::Member { base, .. } => expr_uses_local(base, name),
        ExprKind::Cond {
            cond,
            then_branch,
            else_branch,
        } => {
            expr_uses_local(cond, name)
                || expr_uses_local(then_branch, name)
                || expr_uses_local(else_branch, name)
        }
        ExprKind::InitList(elems) => elems.iter().any(|e| expr_uses_local(e, name)),
        ExprKind::Call { args, .. } => args.iter().any(|e| expr_uses_local(e, name)),
        ExprKind::Cast { expr, .. } | ExprKind::Paren(expr) => expr_uses_local(expr, name),
        ExprKind::IntLit(_)
        | ExprKind::FloatLit(_)
        | ExprKind::StringLit(_)
        | ExprKind::CharLit(_)
        | ExprKind::BoolLit(_)
        | ExprKind::Null
        | ExprKind::Global(_)
        | ExprKind::EnumConst { .. }
        | ExprKind::Unsupported(_) => false,
    }
}

fn is_unit(program: &Program, id: TypeId) -> bool {
    matches!(program.types.canonical(id), Some(Type::Void))
}

fn is_bool(program: &Program, id: TypeId) -> bool {
    matches!(program.types.canonical(id), Some(Type::Bool))
}

fn is_size_t(program: &Program, id: TypeId) -> bool {
    match program.types.get(id) {
        Some(Type::Named { name, .. }) if name == "size_t" => true,
        Some(ty) => matches!(
            ty,
            Type::Int { bits, signed: false, .. } if u16::from(*bits) == u16::from(program.target.size_t_bits)
        ),
        None => false,
    }
}

fn is_slice_element(program: &Program, id: TypeId) -> bool {
    matches!(
        program.types.get(id),
        Some(Type::Bool)
            | Some(Type::Char { .. })
            | Some(Type::Int { .. })
            | Some(Type::Float { .. })
    )
}

fn is_unsigned(program: &Program, id: TypeId) -> bool {
    match program.types.canonical(id) {
        Some(Type::Int { signed: false, .. }) => true,
        Some(Type::Enum { name }) => program
            .enum_named(name)
            .and_then(|item| program.types.canonical(item.underlying))
            .is_some_and(|ty| matches!(ty, Type::Int { signed: false, .. })),
        _ => false,
    }
}

fn same_type(program: &Program, left: TypeId, right: TypeId) -> bool {
    program.types.canonical(left) == program.types.canonical(right)
}
