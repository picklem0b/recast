//! TIR validation.
//!
//! Invariants 1 to 8 are enforced on the M0 subset. Invariants 9 and 10 cover
//! fact tables and configuration formulas, which first appear in later
//! milestones; the checks are documented here so they are not silently lost.

use std::collections::BTreeSet;
use std::fmt;

use crate::id::{ItemId, NodeId, TypeId};
use crate::program::{
    Block, Callee, Expr, ExprKind, Function, Global, Program, Provenance, Stmt, StmtKind, Target,
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
    switch_depth: usize,
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
            switch_depth: 0,
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
        for index in 0..self.program.records.len() {
            self.check_record(index);
        }
        for index in 0..self.program.enums.len() {
            self.check_enum(index);
        }
        for index in 0..self.program.globals.len() {
            self.check_global(index);
        }
    }

    fn check_record(&mut self, index: usize) {
        let path = format!("records[{index}]");
        let record = &self.program.records[index];
        self.check_item(&record.id, &format!("{path}.id"));
        if record.name.is_empty() {
            self.error(1, format!("{path}.name"), "must not be empty");
        }
        self.check_provenance(&record.provenance, &format!("{path}.span"));
        if record.fields.is_empty() {
            self.error(1, format!("{path}.fields"), "a record needs a field");
        }
        let mut names = BTreeSet::new();
        for (field_index, field) in record.fields.iter().enumerate() {
            if field.name.is_empty() {
                self.error(
                    1,
                    format!("{path}.fields[{field_index}].name"),
                    "must not be empty",
                );
            }
            if !names.insert(field.name.as_str()) {
                self.error(
                    1,
                    format!("{path}.fields[{field_index}].name"),
                    format!("duplicate field {}", field.name),
                );
            }
            self.check_type(field.ty, &format!("{path}.fields[{field_index}].type"));
        }
    }

    fn check_enum(&mut self, index: usize) {
        let path = format!("enums[{index}]");
        let item = &self.program.enums[index];
        self.check_item(&item.id, &format!("{path}.id"));
        if item.name.is_empty() {
            self.error(1, format!("{path}.name"), "must not be empty");
        }
        self.check_provenance(&item.provenance, &format!("{path}.span"));
        self.check_type(item.underlying, &format!("{path}.underlying"));
        let mut names = BTreeSet::new();
        for (variant_index, variant) in item.variants.iter().enumerate() {
            if variant.name.is_empty() {
                self.error(
                    1,
                    format!("{path}.variants[{variant_index}].name"),
                    "must not be empty",
                );
            }
            if !names.insert(variant.name.as_str()) {
                self.error(
                    1,
                    format!("{path}.variants[{variant_index}].name"),
                    format!("duplicate enumerator {}", variant.name),
                );
            }
        }
    }

    fn check_global(&mut self, index: usize) {
        let path = format!("globals[{index}]");
        let global = &self.program.globals[index];
        self.check_item(&global.id, &format!("{path}.id"));
        if global.name.is_empty() {
            self.error(1, format!("{path}.name"), "must not be empty");
        }
        self.check_provenance(&global.provenance, &format!("{path}.span"));
        self.check_type(global.ty, &format!("{path}.type"));
        if let Some(init) = &global.init {
            self.check_const_expr(init, &format!("{path}.init"), global);
        }
    }

    /// Validate an initializer outside a function body. Global initializers
    /// may refer to other globals and enumerators, but not to locals.
    fn check_const_expr(&mut self, expr: &Expr, path: &str, global: &Global) {
        self.check_type(expr.ty, &format!("{path}.type"));
        match &expr.kind {
            ExprKind::Local(name) => self.error(
                1,
                format!("{path}.name"),
                format!("global initializer refers to local {name}"),
            ),
            ExprKind::Global(id) | ExprKind::EnumConst { item: id, .. } => {
                if self.program.global(id).is_none() {
                    self.error(
                        1,
                        format!("{path}.id"),
                        format!("global {id} does not exist"),
                    );
                }
            }
            ExprKind::IntLit(_)
            | ExprKind::FloatLit(_)
            | ExprKind::StringLit(_)
            | ExprKind::CharLit(_)
            | ExprKind::BoolLit(_)
            | ExprKind::Null => {}
            ExprKind::Unary { operand, .. } => self.check_const_expr(operand, path, global),
            ExprKind::Binary { lhs, rhs, .. } | ExprKind::Assign { lhs, rhs, .. } => {
                self.check_const_expr(lhs, path, global);
                self.check_const_expr(rhs, path, global);
            }
            ExprKind::Index { base, index } => {
                self.check_const_expr(base, path, global);
                self.check_const_expr(index, path, global);
            }
            ExprKind::Member { base, .. } | ExprKind::Paren(base) => {
                self.check_const_expr(base, path, global)
            }
            ExprKind::Cond {
                cond,
                then_branch,
                else_branch,
            } => {
                self.check_const_expr(cond, path, global);
                self.check_const_expr(then_branch, path, global);
                self.check_const_expr(else_branch, path, global);
            }
            ExprKind::InitList(elems) => {
                for elem in elems {
                    self.check_const_expr(elem, path, global);
                }
            }
            ExprKind::Call { callee, .. } => {
                if matches!(callee, Callee::Unresolved(name) if name.is_empty()) {
                    self.error(1, path, "call with an empty callee name");
                }
            }
            ExprKind::Cast { to, expr, .. } => {
                self.check_type(*to, path);
                self.check_const_expr(expr, path, global);
            }
            ExprKind::Unsupported(spelling) => {
                self.error(3, path, format!("unsupported initializer {spelling}"))
            }
        }
        let _ = global;
    }

    fn check_item(&mut self, id: &ItemId, path: &str) {
        if !self.items.insert(id.clone()) {
            self.error(1, path, format!("duplicate item identifier {id}"));
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
        self.check_item(&function.id, &format!("{path}.id"));
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
            StmtKind::DoWhile { body, cond } => {
                self.check_block(
                    body,
                    function,
                    scope,
                    loop_depth + 1,
                    nodes,
                    &format!("{path}.body"),
                );
                self.check_condition(cond, function, scope, nodes, &format!("{path}.cond"));
            }
            StmtKind::Switch { scrutinee, cases } => {
                self.check_expr(
                    scrutinee,
                    function,
                    scope,
                    nodes,
                    &format!("{path}.scrutinee"),
                );
                if !self.is_integer(scrutinee.ty) {
                    self.error(7, format!("{path}.scrutinee"), "switch needs an integer");
                }
                if cases.is_empty() {
                    self.error(1, format!("{path}.cases"), "a switch needs a case");
                }
                let mut labels = BTreeSet::new();
                let mut defaults = 0;
                self.switch_depth += 1;
                for (index, case) in cases.iter().enumerate() {
                    let case_path = format!("{path}.cases[{index}]");
                    match case.label {
                        Some(label) if !labels.insert(label) => self.error(
                            1,
                            format!("{case_path}.label"),
                            format!("duplicate case label {label}"),
                        ),
                        None => defaults += 1,
                        Some(_) => {}
                    }
                    self.check_block(
                        &case.body,
                        function,
                        scope,
                        loop_depth,
                        nodes,
                        &format!("{case_path}.body"),
                    );
                }
                self.switch_depth -= 1;
                if defaults > 1 {
                    self.error(1, format!("{path}.cases"), "a switch has one default");
                }
            }
            StmtKind::Break => {
                if loop_depth == 0 && self.switch_depth == 0 {
                    self.error(5, path, "break has no enclosing loop or switch");
                }
            }
            StmtKind::Continue => {
                if loop_depth == 0 {
                    self.error(5, path, "continue has no enclosing loop");
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
            | ExprKind::CharLit(_)
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
            ExprKind::Global(id) => {
                if self.program.global(id).is_none() {
                    self.error(
                        1,
                        format!("{path}.id"),
                        format!("global {id} does not exist"),
                    );
                }
            }
            ExprKind::EnumConst { item, value } => {
                match self.program.enums.iter().find(|e| &e.id == item) {
                    None => self.error(
                        1,
                        format!("{path}.item"),
                        format!("enum {item} does not exist"),
                    ),
                    Some(item) if !item.variants.iter().any(|variant| variant.value == *value) => {
                        self.error(
                            1,
                            format!("{path}.value"),
                            format!(
                                "enum {item} has no enumerator with value {value}",
                                item = item.name
                            ),
                        )
                    }
                    Some(_) => {}
                }
            }
            ExprKind::Member { base, field, arrow } => {
                self.check_expr(base, function, scope, nodes, &format!("{path}.base"));
                if *arrow {
                    self.error(3, path, "pointer member access is outside the M1 subset");
                }
                let record = match self.program.types.canonical(base.ty) {
                    Some(Type::Record { name }) => self.program.record(name),
                    _ => None,
                };
                match record {
                    Some(record) if record.field(field).is_some() => {}
                    Some(record) => self.error(
                        1,
                        format!("{path}.field"),
                        format!("record {} has no field {field}", record.name),
                    ),
                    None => self.error(1, path, "member access on a type that is not a record"),
                }
            }
            ExprKind::Cond {
                cond,
                then_branch,
                else_branch,
            } => {
                self.check_condition(cond, function, scope, nodes, &format!("{path}.cond"));
                self.check_expr(then_branch, function, scope, nodes, &format!("{path}.then"));
                self.check_expr(else_branch, function, scope, nodes, &format!("{path}.else"));
                if self.program.types.canonical(then_branch.ty)
                    != self.program.types.canonical(else_branch.ty)
                {
                    self.error(7, path, "conditional branches need the same type");
                }
            }
            ExprKind::InitList(elems) => match self.program.types.canonical(expr.ty) {
                Some(Type::Record { name }) => {
                    let fields = self
                        .program
                        .record(name)
                        .map(|record| record.fields.len())
                        .unwrap_or(0);
                    if elems.len() != fields {
                        self.error(
                            1,
                            path,
                            format!(
                                "record {name} has {fields} fields but the initializer has {}",
                                elems.len()
                            ),
                        );
                    }
                    for (index, elem) in elems.iter().enumerate() {
                        self.check_expr(
                            elem,
                            function,
                            scope,
                            nodes,
                            &format!("{path}.elems[{index}]"),
                        );
                    }
                }
                Some(Type::Array { len, elem }) => {
                    if elems.len() as u64 != *len {
                        self.error(
                            1,
                            path,
                            format!(
                                "array has {len} elements but the initializer has {}",
                                elems.len()
                            ),
                        );
                    }
                    for (index, item) in elems.iter().enumerate() {
                        self.check_expr(
                            item,
                            function,
                            scope,
                            nodes,
                            &format!("{path}.elems[{index}]"),
                        );
                        if self.program.types.canonical(item.ty)
                            != self.program.types.canonical(*elem)
                        {
                            self.error(
                                7,
                                format!("{path}.elems[{index}]"),
                                "array initializer needs an explicit cast to the element type",
                            );
                        }
                    }
                }
                _ => self.error(1, path, "initializer lists need a record or array type"),
            },
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
            Type::Array { elem, len } => {
                if len == 0 {
                    self.error(3, path, "an array needs a positive length");
                }
                self.check_type_inner(elem, path, seen);
            }
            Type::Record { name } => {
                if self.program.record(&name).is_none() {
                    self.error(3, path, format!("record {name} is not defined"));
                }
            }
            Type::Enum { name } => {
                if self.program.enum_named(&name).is_none() {
                    self.error(3, path, format!("enum {name} is not defined"));
                }
            }
            Type::Named { canonical, .. } => self.check_type_inner(canonical, path, seen),
            Type::Void | Type::Bool | Type::Char { .. } => {}
        }
    }

    fn is_integer(&self, id: TypeId) -> bool {
        matches!(
            self.program.types.canonical(id),
            Some(Type::Bool)
                | Some(Type::Char { .. })
                | Some(Type::Int { .. })
                | Some(Type::Enum { .. })
        )
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
