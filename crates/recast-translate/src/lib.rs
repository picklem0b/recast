//! Translation from TIR to Rust.
//!
//! M0 covers integer functions and the `sum(const int *, size_t)` slice
//! pattern. Slice recognition is deliberately conservative: a pointer
//! parameter is treated as a slice only when the next parameter is a
//! `size_t`, and every use of the pointer must be an index and every use of
//! the length must be a standalone local. Anything else is refused rather
//! than translated unsafely.

pub mod rust;

use std::collections::BTreeSet;
use std::fmt;

use recast_ir::{
    Block, Callee, Expr, ExprKind, Function, Linkage, Program, Stmt, StmtKind, Type, TypeId,
};
use rust::{Expr as RExpr, File as RFile, Fn as RFn, Stmt as RStmt};

/// Generated Rust source for one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    pub source: String,
}

/// A construct M0 cannot translate without guessing.
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

/// Translate every function with a body.
pub fn translate(program: &Program) -> Result<Translation, TranslateError> {
    let mut file = RFile {
        attrs: vec!["#![forbid(unsafe_code)]".to_string()],
        fns: Vec::new(),
    };
    for function in &program.functions {
        if function.body.is_none() {
            continue;
        }
        let translator = Translator::new(program, function)?;
        file.fns.push(translator.function(function)?);
    }
    Ok(Translation {
        source: file.render(),
    })
}

#[derive(Debug, Clone)]
struct SlicePlan {
    pointer: String,
    length: String,
    element: TypeId,
    is_const: bool,
}

struct Translator<'a> {
    program: &'a Program,
    slices: Vec<SlicePlan>,
    function: &'a str,
    ret: TypeId,
    assigned: BTreeSet<String>,
}

impl<'a> Translator<'a> {
    fn new(program: &'a Program, function: &'a Function) -> Result<Self, TranslateError> {
        let mut slices = Vec::new();
        for window in function.signature.params.windows(2) {
            let pointer = &window[0];
            let length = &window[1];
            let Some(pointer_name) = &pointer.name else {
                continue;
            };
            let Some(length_name) = &length.name else {
                continue;
            };
            let Some(Type::Pointer { pointee, is_const }) = program.types.get(pointer.ty) else {
                continue;
            };
            if !is_size_t(program, length.ty) {
                continue;
            }
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

        let mut assigned = BTreeSet::new();
        if let Some(body) = &function.body {
            collect_assigned(body, &mut assigned);
        }

        Ok(Self {
            program,
            slices,
            function: &function.name,
            ret: function.signature.ret,
            assigned,
        })
    }

    fn function(&self, function: &Function) -> Result<RFn, TranslateError> {
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
            out.extend(self.stmt(stmt)?);
        }
        if returns_value {
            match block.stmts.last() {
                Some(Stmt {
                    kind: StmtKind::Return(Some(_)),
                    ..
                }) => {}
                _ => {
                    return Err(
                        self.error("a non-void function must end in a return for M0 translation")
                    )
                }
            }
        }
        Ok(out)
    }

    fn stmt(&self, stmt: &Stmt) -> Result<Vec<RStmt>, TranslateError> {
        let single = match &stmt.kind {
            StmtKind::Decl { name, ty, init } => vec![RStmt::Let {
                name: name.clone(),
                mutable: self.assigned.contains(name),
                ty: self.rust_type(*ty)?,
                init: match init {
                    Some(init) => Some(self.value(init, *ty)?),
                    None => None,
                },
            }],
            StmtKind::Expr(expr) => vec![self.expr_statement(expr)?],
            StmtKind::If {
                cond,
                then_branch,
                else_branch,
            } => vec![RStmt::If {
                cond: self.condition(cond)?,
                then_body: self.nested(then_branch)?,
                else_body: match else_branch {
                    Some(block) => Some(self.nested(block)?),
                    None => None,
                },
            }],
            StmtKind::While { cond, body } => vec![RStmt::While {
                cond: self.condition(cond)?,
                body: self.nested(body)?,
            }],
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                let mut out = Vec::new();
                if let Some(init) = init {
                    out.extend(self.stmt(init)?);
                }
                let cond = match cond {
                    Some(cond) => self.condition(cond)?,
                    None => RExpr::Bool(true),
                };
                let mut body = self.nested(body)?;
                if let Some(step) = step {
                    body.push(self.expr_statement(step)?);
                }
                out.push(RStmt::While { cond, body });
                out
            }
            StmtKind::Return(value) => vec![RStmt::Return(match value {
                Some(value) => Some(self.value(value, self.ret)?),
                None => None,
            })],
            StmtKind::Break => vec![RStmt::Break],
            StmtKind::Continue => vec![RStmt::Continue],
            StmtKind::Empty => Vec::new(),
        };
        Ok(single)
    }

    fn nested(&self, block: &Block) -> Result<Vec<RStmt>, TranslateError> {
        let mut out = Vec::new();
        for stmt in &block.stmts {
            out.extend(self.stmt(stmt)?);
        }
        Ok(out)
    }

    fn expr_statement(&self, expr: &Expr) -> Result<RStmt, TranslateError> {
        match &expr.kind {
            ExprKind::Assign { op, lhs, rhs } => Ok(RStmt::Assign {
                target: self.expr(lhs)?,
                op: match op.as_str() {
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
                        return Err(
                            self.error(format!("assignment operator {other} is not supported"))
                        )
                    }
                },
                value: self.value(rhs, lhs.ty)?,
            }),
            ExprKind::Unary {
                op,
                operand,
                postfix: _,
            } if op == "++" || op == "--" => Ok(RStmt::Assign {
                target: self.expr(operand)?,
                op: if op == "++" { "+=" } else { "-=" },
                value: RExpr::Int(1),
            }),
            _ => Ok(RStmt::Expr(self.expr(expr)?)),
        }
    }

    fn expr(&self, expr: &Expr) -> Result<RExpr, TranslateError> {
        match &expr.kind {
            ExprKind::IntLit(value) => Ok(RExpr::Int(*value)),
            ExprKind::FloatLit(value) => Ok(RExpr::Float(*value)),
            ExprKind::BoolLit(value) => Ok(RExpr::Bool(*value)),
            ExprKind::Local(name) => {
                if let Some(plan) = self.slice_by_length(name) {
                    return Ok(RExpr::Method {
                        receiver: Box::new(RExpr::Path(plan.pointer.clone())),
                        method: "len".to_string(),
                        args: Vec::new(),
                    });
                }
                if self.slice_by_pointer(name).is_some() {
                    return Err(self.error(format!(
                        "pointer parameter {name} is only translated where it is indexed"
                    )));
                }
                Ok(RExpr::Path(name.clone()))
            }
            ExprKind::Unary { op, operand, .. } => match op.as_str() {
                "-" => Ok(RExpr::Unary {
                    op: "-",
                    expr: Box::new(self.value(operand, operand.ty)?),
                }),
                "~" => Ok(RExpr::Unary {
                    op: "!",
                    expr: Box::new(self.value(operand, operand.ty)?),
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
                let plan = self.index_plan(base)?;
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
                    base: Box::new(RExpr::Path(plan.pointer.clone())),
                    index: Box::new(index),
                })
            }
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
            ExprKind::StringLit(_) | ExprKind::Null | ExprKind::Unsupported(_) => {
                Err(self.error(format!(
                    "expression {:?} is outside the M0 translation subset",
                    expr.kind
                )))
            }
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
            "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" => {
                let lhs_expr = self.value(lhs, lhs.ty)?;
                let rhs_expr = self.value(rhs, rhs.ty)?;
                if matches!(op, "+" | "-" | "*") && is_unsigned(self.program, lhs.ty) {
                    let method = match op {
                        "+" => "wrapping_add",
                        "-" => "wrapping_sub",
                        _ => "wrapping_mul",
                    };
                    return Ok(RExpr::Method {
                        receiver: Box::new(lhs_expr),
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
                        "^" => "^",
                        "<<" => "<<",
                        _ => ">>",
                    },
                    lhs: Box::new(lhs_expr),
                    rhs: Box::new(rhs_expr),
                })
            }
            other => Err(self.error(format!("binary operator {other} is not supported"))),
        }
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
                rhs: Box::new(RExpr::Int(0)),
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
            _ => is_bool(self.program, expr.ty),
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
            return Ok(RExpr::Int(*value));
        }
        let target = self.rust_type(to)?;
        Ok(RExpr::Cast {
            expr: Box::new(self.expr(expr)?),
            ty: target,
        })
    }

    fn index_plan(&self, base: &Expr) -> Result<&SlicePlan, TranslateError> {
        let mut current = base;
        loop {
            match &current.kind {
                ExprKind::Paren(inner) | ExprKind::Cast { expr: inner, .. } => current = inner,
                ExprKind::Local(name) => {
                    return self.slice_by_pointer(name).ok_or_else(|| {
                        self.error(format!(
                            "indexing {name} is only supported when it is a slice parameter"
                        ))
                    })
                }
                _ => return Err(self.error("unsupported index base")),
            }
        }
    }

    fn rust_type(&self, id: TypeId) -> Result<String, TranslateError> {
        let rendered = match self.program.types.get(id) {
            Some(Type::Void) => "()".to_string(),
            Some(Type::Bool) => "bool".to_string(),
            Some(Type::Char { signed }) => {
                if *signed {
                    "i8".to_string()
                } else {
                    "u8".to_string()
                }
            }
            Some(Type::Int { bits, signed, .. }) => {
                format!("{}{}", if *signed { 'i' } else { 'u' }, bits)
            }
            Some(Type::Float { bits, .. }) => format!("f{bits}"),
            Some(Type::Named { name, canonical }) => {
                if name == "size_t" {
                    "usize".to_string()
                } else {
                    return self.rust_type(*canonical);
                }
            }
            Some(Type::Pointer { .. }) => return Err(self.error(
                "a pointer type is only translated as a slice with a following size_t parameter",
            )),
            Some(Type::Unknown { spelling }) => {
                return Err(self.error(format!("unknown type {spelling}")))
            }
            None => return Err(self.error(format!("missing type {id}"))),
        };
        Ok(rendered)
    }

    fn slice_by_pointer(&self, name: &str) -> Option<&SlicePlan> {
        self.slices.iter().find(|plan| plan.pointer == name)
    }

    fn slice_by_length(&self, name: &str) -> Option<&SlicePlan> {
        self.slices.iter().find(|plan| plan.length == name)
    }

    fn error(&self, message: impl Into<String>) -> TranslateError {
        TranslateError::new(self.function, message)
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
        StmtKind::Return(Some(value)) => collect_expr_assignments(value, names),
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Empty => {}
    }
}

fn collect_expr_assignments(expr: &Expr, names: &mut BTreeSet<String>) {
    match &expr.kind {
        ExprKind::Assign { lhs, rhs, .. } => {
            if let Some(name) = local_name(lhs) {
                names.insert(name.to_string());
            }
            collect_expr_assignments(lhs, names);
            collect_expr_assignments(rhs, names);
        }
        ExprKind::Unary { op, operand, .. } => {
            if op == "++" || op == "--" {
                if let Some(name) = local_name(operand) {
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
        | ExprKind::BoolLit(_)
        | ExprKind::Null
        | ExprKind::Local(_)
        | ExprKind::Unsupported(_) => {}
    }
}

fn local_name(expr: &Expr) -> Option<&str> {
    match &expr.kind {
        ExprKind::Local(name) => Some(name),
        ExprKind::Paren(inner) | ExprKind::Cast { expr: inner, .. } => local_name(inner),
        _ => None,
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
    matches!(
        program.types.canonical(id),
        Some(Type::Int { signed: false, .. })
    )
}

fn same_type(program: &Program, left: TypeId, right: TypeId) -> bool {
    program.types.canonical(left) == program.types.canonical(right)
}
