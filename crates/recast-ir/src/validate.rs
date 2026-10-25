//! TIR validation.
//!
//! Invariants 1 to 8 are enforced on the M0 subset. Invariants 9 and 10 cover
//! fact tables and configuration formulas, which first appear in later
//! milestones; the checks are documented here so they are not silently lost.

use std::collections::BTreeSet;
use std::fmt;

use crate::id::{ItemId, NodeId, TypeId};
use crate::program::{
    Block, Callee, Expr, ExprKind, Function, Program, Provenance, Stmt, StmtKind, Target,
    SCHEMA_VERSION,
};
use crate::types::Type;

/// A validation failure tied to the invariant that caught it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// Invariant number from document 04 section 10.
    pub invariant: u8,
    pub path: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(invariant: u8, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            invariant,
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invariant {} at {}: {}",
            self.invariant, self.path, self.message
        )
    }
}

impl std::error::Error for ValidationError {}

/// Validate a program and return every failure found.
pub fn validate(program: &Program) -> Result<(), Vec<ValidationError>> {
    let mut validator = Validator::new(program);
    validator.run();
    if validator.errors.is_empty() {
        Ok(())
    } else {
        Err(validator.errors)
    }
}

struct Validator<'a> {
    program: &'a Program,
    errors: Vec<ValidationError>,
    files: BTreeSet<&'a str>,
    items: BTreeSet<ItemId>,
}

impl<'a> Validator<'a> {
    fn new(program: &'a Program) -> Self {
        let mut files = BTreeSet::new();
        let mut errors = Vec::new();
        for file in &program.files {
            if !files.insert(file.path.as_str()) {
                errors.push(ValidationError::new(
                    1,
                    "files",
                    format!("duplicate source file {}", file.path),
                ));
            }
        }
        Self {
            program,
            errors,
            files,
            items: BTreeSet::new(),
        }
    }

    fn error(&mut self, invariant: u8, path: impl Into<String>, message: impl Into<String>) {
        self.errors
            .push(ValidationError::new(invariant, path, message));
    }

    fn run(&mut self) {
        if self.program.schema_version != SCHEMA_VERSION {
            self.error(
                1,
                "schema_version",
                format!(
                    "expected {SCHEMA_VERSION}, found {}",
                    self.program.schema_version
                ),
            );
        }
        self.check_target();

        for index in 0..self.program.functions.len() {
            self.check_function(index);
        }
    }

    fn check_target(&mut self) {
        let Target {
            triple,
            pointer_bits,
            int_bits,
            long_bits,
            long_long_bits,
            endian: _,
            max_align: _,
            size_t_bits,
            ..
        } = &self.program.target;
        if triple.is_empty() {
            self.error(1, "target.triple", "must not be empty");
        }
        if !matches!(pointer_bits, 32 | 64) {
            self.error(1, "target.pointer_bits", "must be 32 or 64");
        }
        if !matches!(int_bits, 16 | 32 | 64) {
            self.error(1, "target.int_bits", "must be 16, 32 or 64");
        }
        if !matches!(long_bits, 32 | 64) {
            self.error(1, "target.long_bits", "must be 32 or 64");
        }
        if *long_long_bits != 64 {
            self.error(1, "target.long_long_bits", "must be 64");
        }
        if !matches!(size_t_bits, 32 | 64) {
            self.error(1, "target.size_t_bits", "must be 32 or 64");
        }
    }

    fn check_function(&mut self, index: usize) {
        let path = format!("functions[{index}]");
        let function = &self.program.functions[index];
        if !self.items.insert(function.id.clone()) {
            self.error(
                1,
                format!("{path}.id"),
                format!("duplicate item identifier {}", function.id),
            );
        }
        if function.name.is_empty() {
            self.error(1, format!("{path}.name"), "must not be empty");
        }
        self.check_provenance(&function.provenance, &format!("{path}.span"));
        self.check_type(function.signature.ret, &format!("{path}.ret"));
        for (param_index, param) in function.signature.params.iter().enumerate() {
            self.check_type(param.ty, &format!("{path}.params[{param_index}].type"));
        }

        let mut scope = Scope::default();
        for param in &function.signature.params {
            if let Some(name) = &param.name {
                scope.names.insert(name.clone());
            }
        }

        if let Some(body) = &function.body {
            let mut nodes = BTreeSet::new();
            self.check_block(
                body,
                function,
                &scope,
                0,
                &mut nodes,
                &format!("{path}.body"),
            );
            if !nodes.is_empty() {
                let max = *nodes.iter().next_back().expect("non-empty");
                if nodes.len() != max as usize + 1 {
                    self.error(
                        1,
                        format!("{path}.body"),
                        "node ordinals must be contiguous from zero",
                    );
                }
            }
        }
    }

    fn check_block(
        &mut self,
        block: &Block,
        function: &Function,
        scope: &Scope,
        loop_depth: usize,
        nodes: &mut BTreeSet<u32>,
        path: &str,
    ) {
        let mut scope = scope.clone();
        for (index, stmt) in block.stmts.iter().enumerate() {
            self.check_stmt(
                stmt,
                function,
                &mut scope,
                loop_depth,
                nodes,
                &format!("{path}.stmts[{index}]"),
            );
        }
    }

    fn check_stmt(
        &mut self,
        stmt: &Stmt,
        function: &Function,
        scope: &mut Scope,
        loop_depth: usize,
        nodes: &mut BTreeSet<u32>,
        path: &str,
    ) {
        self.check_node(&stmt.id, function, nodes, path);
        self.check_provenance(&stmt.provenance, &format!("{path}.span"));

        match &stmt.kind {
            StmtKind::Decl { name, ty, init } => {
                if name.is_empty() {
                    self.error(1, format!("{path}.name"), "must not be empty");
                }
                self.check_type(*ty, &format!("{path}.type"));
                if let Some(init) = init {
                    self.check_expr(init, function, scope, nodes, &format!("{path}.init"));
                    let declared = self.program.types.canonical(*ty);
                    let initializer = self.program.types.canonical(init.ty);
                    if declared != initializer {
                        self.error(
                            7,
                            format!("{path}.init"),
                            "initializer needs an explicit cast to the declared type",
                        );
                    }
                }
                scope.names.insert(name.clone());
            }
            StmtKind::Expr(expr) => {
                self.check_expr(expr, function, scope, nodes, &format!("{path}.expr"));
            }
            StmtKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.check_condition(cond, function, scope, nodes, &format!("{path}.cond"));
                self.check_block(
                    then_branch,
                    function,
                    scope,
                    loop_depth,
                    nodes,
                    &format!("{path}.then_branch"),
                );
                if let Some(else_branch) = else_branch {
                    self.check_block(
                        else_branch,
                        function,
                        scope,
                        loop_depth,
                        nodes,
                        &format!("{path}.else_branch"),
                    );
                }
            }
            StmtKind::While { cond, body } => {
                self.check_condition(cond, function, scope, nodes, &format!("{path}.cond"));
                self.check_block(
                    body,
                    function,
                    scope,
                    loop_depth + 1,
                    nodes,
                    &format!("{path}.body"),
                );
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                let mut loop_scope = scope.clone();
                if let Some(init) = init {
                    self.check_stmt(
                        init,
                        function,
                        &mut loop_scope,
                        loop_depth,
                        nodes,
                        &format!("{path}.init"),
                    );
                }
                if let Some(cond) = cond {
                    self.check_condition(
                        cond,
                        function,
                        &loop_scope,
                        nodes,
                        &format!("{path}.cond"),
                    );
                }
                if let Some(step) = step {
                    self.check_expr(step, function, &loop_scope, nodes, &format!("{path}.step"));
                }
                self.check_block(
                    body,
                    function,
                    &loop_scope,
                    loop_depth + 1,
                    nodes,
                    &format!("{path}.body"),
                );
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.check_expr(value, function, scope, nodes, &format!("{path}.value"));
                    let returned = self.program.types.canonical(value.ty);
                    let expected = self.program.types.canonical(function.signature.ret);
                    if returned != expected {
                        self.error(
                            7,
                            format!("{path}.value"),
                            "return value needs an explicit cast to the return type",
                        );
                    }
                } else if self.program.types.canonical(function.signature.ret) != Some(&Type::Void)
                {
                    self.error(7, path, "non-void function returns without a value");
                }
            }
            StmtKind::Break | StmtKind::Continue => {
                if loop_depth == 0 {
                    self.error(5, path, "break or continue has no enclosing loop");
                }
            }
            StmtKind::Empty => {}
        }
    }

    fn check_expr(
        &mut self,
        expr: &Expr,
        function: &Function,
        scope: &Scope,
        nodes: &mut BTreeSet<u32>,
        path: &str,
    ) {
        self.check_node(&expr.id, function, nodes, path);
        self.check_provenance(&expr.provenance, &format!("{path}.span"));
        self.check_type(expr.ty, &format!("{path}.type"));

        match &expr.kind {
            ExprKind::IntLit(_)
            | ExprKind::FloatLit(_)
            | ExprKind::StringLit(_)
            | ExprKind::BoolLit(_)
            | ExprKind::Null => {}
            ExprKind::Local(name) => {
                if !scope.names.contains(name) {
                    self.error(
                        1,
                        format!("{path}.name"),
                        format!("local {name} is not declared in this scope"),
                    );
                }
            }
            ExprKind::Unary { operand, .. } => {
                self.check_expr(operand, function, scope, nodes, &format!("{path}.operand"));
            }
            ExprKind::Binary { op, lhs, rhs } => {
                self.check_expr(lhs, function, scope, nodes, &format!("{path}.lhs"));
                self.check_expr(rhs, function, scope, nodes, &format!("{path}.rhs"));
                if binary_needs_equal_operands(op)
                    && self.program.types.canonical(lhs.ty) != self.program.types.canonical(rhs.ty)
                {
                    self.error(
                        7,
                        path,
                        format!("operands of {op} need an explicit implicit cast"),
                    );
                }
            }
            ExprKind::Assign { lhs, rhs, .. } => {
                self.check_expr(lhs, function, scope, nodes, &format!("{path}.lhs"));
                self.check_expr(rhs, function, scope, nodes, &format!("{path}.rhs"));
                if self.program.types.canonical(lhs.ty) != self.program.types.canonical(rhs.ty) {
                    self.error(
                        7,
                        path,
                        "assignment needs an explicit cast to the destination type",
                    );
                }
            }
            ExprKind::Index { base, index } => {
                self.check_expr(base, function, scope, nodes, &format!("{path}.base"));
                self.check_expr(index, function, scope, nodes, &format!("{path}.index"));
            }
            ExprKind::Call { callee, args } => {
                match callee {
                    Callee::Direct(id) => {
                        let signature = self.program.function(id).map(|function| {
                            (function.signature.params.len(), function.signature.variadic)
                        });
                        match signature {
                            None => {
                                self.error(
                                    1,
                                    format!("{path}.callee"),
                                    format!("call target {id} does not exist"),
                                );
                            }
                            Some((arity, variadic)) => {
                                let matches = if variadic {
                                    args.len() >= arity
                                } else {
                                    args.len() == arity
                                };
                                if !matches {
                                    self.error(
                                        4,
                                        path,
                                        format!(
                                            "call to {id} has {} arguments, expected {arity}",
                                            args.len()
                                        ),
                                    );
                                }
                            }
                        }
                    }
                    Callee::Indirect(callee) => {
                        self.check_expr(callee, function, scope, nodes, &format!("{path}.callee"));
                    }
                    Callee::Unresolved(name) => {
                        self.error(
                            4,
                            format!("{path}.callee"),
                            format!("unresolved callee {name} cannot enter the M0 pipeline"),
                        );
                    }
                }
                for (index, arg) in args.iter().enumerate() {
                    self.check_expr(
                        arg,
                        function,
                        scope,
                        nodes,
                        &format!("{path}.args[{index}]"),
                    );
                }
            }
            ExprKind::Cast { to, expr, .. } => {
                self.check_type(*to, &format!("{path}.to"));
                self.check_expr(expr, function, scope, nodes, &format!("{path}.expr"));
            }
            ExprKind::Paren(expr) => {
                self.check_expr(expr, function, scope, nodes, &format!("{path}.expr"));
            }
            ExprKind::Unsupported(spelling) => {
                self.error(
                    3,
                    path,
                    format!("unsupported expression {spelling} cannot enter the M0 pipeline"),
                );
            }
        }
    }

    fn check_condition(
        &mut self,
        expr: &Expr,
        function: &Function,
        scope: &Scope,
        nodes: &mut BTreeSet<u32>,
        path: &str,
    ) {
        self.check_expr(expr, function, scope, nodes, path);
        if self.program.types.canonical(expr.ty) != Some(&Type::Bool) {
            self.error(7, path, "condition needs an explicit implicit cast to bool");
        }
    }

    fn check_node(
        &mut self,
        node: &NodeId,
        function: &Function,
        nodes: &mut BTreeSet<u32>,
        path: &str,
    ) {
        if node.item() != &function.id {
            self.error(
                1,
                format!("{path}.id"),
                format!("node {node} belongs to another item"),
            );
        }
        if !nodes.insert(node.ordinal()) {
            self.error(
                1,
                format!("{path}.id"),
                format!(
                    "duplicate node ordinal {} in {}",
                    node.ordinal(),
                    function.id
                ),
            );
        }
    }

    fn check_provenance(&mut self, provenance: &Provenance, path: &str) {
        if provenance.span.line == 0 {
            if !provenance.synthesized {
                self.error(8, path, "source node has no line");
            }
            return;
        }
        if !self.files.contains(provenance.span.file.as_str()) {
            self.error(
                8,
                path,
                format!("span refers to unknown file {}", provenance.span.file),
            );
        }
    }

    fn check_type(&mut self, id: TypeId, path: &str) {
        let mut seen = BTreeSet::new();
        self.check_type_inner(id, path, &mut seen);
    }

    fn check_type_inner(&mut self, id: TypeId, path: &str, seen: &mut BTreeSet<TypeId>) {
        if !seen.insert(id) {
            self.error(1, path, format!("type cycle at {id}"));
            return;
        }
        let ty = match self.program.types.get(id) {
            Some(ty) => ty.clone(),
            None => {
                self.error(1, path, format!("type {id} does not exist"));
                return;
            }
        };
        match ty {
            Type::Unknown { spelling } => {
                self.error(
                    3,
                    path,
                    format!("unknown type {spelling} cannot enter the M0 pipeline"),
                );
            }
            Type::Int { bits, .. } => {
                if !matches!(bits, 8 | 16 | 32 | 64 | 128) {
                    self.error(3, path, format!("integer width {bits} is not supported"));
                }
            }
            Type::Float { bits, .. } => {
                if !matches!(bits, 32 | 64 | 80 | 128) {
                    self.error(3, path, format!("float width {bits} is not supported"));
                }
            }
            Type::Pointer { pointee, .. } => self.check_type_inner(pointee, path, seen),
            Type::Named { canonical, .. } => self.check_type_inner(canonical, path, seen),
            Type::Void | Type::Bool | Type::Char { .. } => {}
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Scope {
    names: BTreeSet<String>,
}

fn binary_needs_equal_operands(op: &str) -> bool {
    matches!(
        op,
        "+" | "-"
            | "*"
            | "/"
            | "%"
            | "&"
            | "|"
            | "^"
            | "<"
            | "<="
            | ">"
            | ">="
            | "=="
            | "!="
            | "&&"
            | "||"
    )
}

// Invariant 9: fact tables reference only existing keys. M0 has no fact
// tables; analysis supplies them at M2.
// Invariant 10: configuration formulas are well formed. M0 has no formulas;
// the preprocessor supplies them at M4.
