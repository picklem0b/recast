//! Lowering from the exporter schema to TIR.
//!
//! M1 accepts the L0 subset: scalar, pointer, array and record types, structs,
//! enums, constants, functions with a body, and the full statement set except
//! `goto`. Anything outside that subset is rejected by schema validation
//! before it reaches this crate.
//!
//! Lowering is deterministic. Source files are sorted by path, functions by
//! name, and node identifiers are handed out in pre-order per function, so the
//! same export always produces the same program and the same dump.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use recast_ir::{
    Block, Callee, Case, Endian, EnumDef, Enumerator, Expr, ExprKind, FieldDef, FnSig, Function,
    Global, ItemId, Linkage, NodeId, Param, Program, Provenance, RecordDef, Span, Stmt, StmtKind,
    Target, Type, TypeId, ValueCategory,
};
use recast_schema as schema;

/// A lowering failure that names the schema path that caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendError {
    pub path: String,
    pub message: String,
}

impl FrontendError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for FrontendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for FrontendError {}

/// Lower one validated export to TIR.
///
/// The caller is expected to run [`Program::validate`] afterwards; `lower`
/// performs only the structural checks it needs to build identifiers.
pub fn lower(ast: &schema::Ast) -> Result<Program, FrontendError> {
    let mut program = Program::new(lower_target(&ast.target)?);

    let mut files: Vec<&str> = ast
        .translation_units
        .iter()
        .map(|unit| unit.file.as_str())
        .collect();
    files.sort_unstable();
    for file in files {
        program.add_file(file);
    }

    let mut functions: Vec<(&schema::Function, &str)> = Vec::new();
    for unit in &ast.translation_units {
        for function in &unit.functions {
            functions.push((function, unit.file.as_str()));
        }
    }
    functions.sort_by(|left, right| left.0.name.cmp(&right.0.name));

    // Records and enums are interned before any function so field and
    // parameter types can name them, and globals before functions so bodies
    // can refer to constants.
    let mut lowerer = Lowerer {
        program,
        items: BTreeMap::new(),
        globals: BTreeMap::new(),
        enumerators: BTreeMap::new(),
        locals: BTreeSet::new(),
    };

    for unit in &ast.translation_units {
        for record in &unit.records {
            lowerer.lower_record(record, &unit.file)?;
        }
        for item in &unit.enums {
            lowerer.lower_enum(item, &unit.file)?;
        }
    }
    for unit in &ast.translation_units {
        for global in &unit.globals {
            lowerer.lower_global(global, &unit.file)?;
        }
    }

    let mut items = BTreeMap::new();
    for (function, _) in &functions {
        items.insert(function.name.clone(), ItemId::function(&function.name));
    }
    lowerer.items = items.clone();

    let mut lowered = Vec::with_capacity(functions.len());
    for (function, file) in &functions {
        lowered.push(lowerer.lower_function(function, file, &items[&function.name])?);
    }
    for function in lowered {
        lowerer.program.add_function(function);
    }

    Ok(lowerer.program)
}

fn lower_target(target: &schema::Target) -> Result<Target, FrontendError> {
    let endian = match target.endian.as_str() {
        "little" => Endian::Little,
        "big" => Endian::Big,
        other => {
            return Err(FrontendError::new(
                "target.endian",
                format!("unknown byte order {other}"),
            ))
        }
    };
    Ok(Target {
        triple: target.triple.clone(),
        pointer_bits: target.pointer_bits,
        int_bits: target.int_bits,
        long_bits: target.long_bits,
        long_long_bits: target.long_long_bits,
        char_is_signed: target.char_signed,
        endian,
        max_align: 16,
        size_t_bits: target.pointer_bits,
    })
}

struct Lowerer {
    program: Program,
    items: BTreeMap<String, ItemId>,
    globals: BTreeMap<String, ItemId>,
    enumerators: BTreeMap<String, (ItemId, i64)>,
    locals: BTreeSet<String>,
}

impl Lowerer {
    fn lower_record(&mut self, record: &schema::Record, file: &str) -> Result<(), FrontendError> {
        let id = ItemId::record(&record.name);
        self.items.insert(record.name.clone(), id.clone());
        let name = record.name.clone();
        // Intern the record type before its fields so a field may name it.
        self.program
            .types
            .intern(Type::Record { name: name.clone() });
        let mut fields = Vec::with_capacity(record.fields.len());
        for field in &record.fields {
            fields.push(FieldDef {
                name: field.name.clone(),
                ty: self.lower_type(&field.ty),
            });
        }
        self.program.records.push(RecordDef {
            id,
            name,
            fields,
            size: record.size,
            align: record.align,
            provenance: Provenance::source(span_of_raw(&record.span, file)),
        });
        Ok(())
    }

    fn lower_enum(&mut self, item: &schema::EnumDef, file: &str) -> Result<(), FrontendError> {
        let id = ItemId::enumerated(&item.name);
        self.items.insert(item.name.clone(), id.clone());
        self.program.types.intern(Type::Enum {
            name: item.name.clone(),
        });
        let underlying = self.lower_type(&item.underlying);
        let mut variants = Vec::with_capacity(item.variants.len());
        for variant in &item.variants {
            self.enumerators
                .insert(variant.name.clone(), (id.clone(), variant.value));
            variants.push(Enumerator {
                name: variant.name.clone(),
                value: variant.value,
            });
        }
        self.program.enums.push(EnumDef {
            id,
            name: item.name.clone(),
            underlying,
            variants,
            provenance: Provenance::source(span_of_raw(&item.span, file)),
        });
        Ok(())
    }

    fn lower_global(&mut self, global: &schema::Global, file: &str) -> Result<(), FrontendError> {
        let id = ItemId::global(&global.name);
        self.globals.insert(global.name.clone(), id.clone());
        let ty = self.lower_type(&global.ty);
        let linkage = match global.linkage.as_str() {
            "external" => Linkage::External,
            "internal" => Linkage::Internal,
            other => {
                return Err(FrontendError::new(
                    format!("globals[{}].linkage", global.name),
                    format!("unknown linkage {other}"),
                ))
            }
        };
        let init = match &global.init {
            Some(init) => {
                let saved = std::mem::take(&mut self.locals);
                let mut nodes = NodeIds::new(id.clone());
                let lowered = self.lower_expr(init, &mut nodes);
                self.locals = saved;
                Some(lowered?)
            }
            None => None,
        };
        self.program.globals.push(Global {
            id,
            name: global.name.clone(),
            ty,
            init,
            is_const: global.is_const,
            linkage,
            provenance: Provenance::source(span_of_raw(&global.span, file)),
        });
        Ok(())
    }

    fn lower_function(
        &mut self,
        function: &schema::Function,
        file: &str,
        id: &ItemId,
    ) -> Result<Function, FrontendError> {
        let ret = self.lower_type(&function.return_type);
        let mut params = Vec::with_capacity(function.params.len());
        self.locals.clear();
        for param in &function.params {
            if let Some(name) = &param.name {
                self.locals.insert(name.clone());
            }
            params.push(Param {
                name: param.name.clone(),
                ty: self.lower_type(&param.ty),
            });
        }
        let linkage = match function.linkage.as_str() {
            "external" => Linkage::External,
            "internal" => Linkage::Internal,
            other => {
                return Err(FrontendError::new(
                    format!("functions[{}].linkage", function.name),
                    format!("unknown linkage {other}"),
                ))
            }
        };
        let span = span_of(function, file);
        let body = match &function.body {
            Some(body) => {
                let mut nodes = NodeIds::new(id.clone());
                Some(self.lower_block(body, file, &mut nodes)?)
            }
            None => None,
        };

        Ok(Function {
            id: id.clone(),
            name: function.name.clone(),
            linkage,
            signature: FnSig {
                params,
                ret,
                variadic: false,
            },
            body,
            provenance: Provenance::source(span),
        })
    }

    fn lower_type(&mut self, ty: &schema::Ty) -> TypeId {
        let lowered = match ty {
            schema::Ty::Void => Type::Void,
            schema::Ty::Bool => Type::Bool,
            schema::Ty::Int {
                bits,
                signed,
                spelling,
            } => {
                let spelling = normalize_spelling(spelling);
                if spelling == "char" {
                    Type::Char { signed: *signed }
                } else {
                    Type::int(*bits, *signed, spelling)
                }
            }
            schema::Ty::Float { bits, spelling } => Type::Float {
                bits: u16::from(*bits),
                spelling: normalize_spelling(spelling),
            },
            schema::Ty::Pointer {
                pointee,
                pointee_const,
            } => {
                let pointee = self.lower_type(pointee);
                Type::Pointer {
                    pointee,
                    is_const: *pointee_const,
                }
            }
            schema::Ty::Array { elem, len } => {
                let elem = self.lower_type(elem);
                Type::Array { elem, len: *len }
            }
            schema::Ty::Record { name } => Type::Record { name: name.clone() },
            schema::Ty::Enum { name } => Type::Enum { name: name.clone() },
            schema::Ty::Named { name, canonical } => {
                let canonical = self.lower_type(canonical);
                if name == "size_t" {
                    Type::size_t(canonical)
                } else {
                    Type::Named {
                        name: name.clone(),
                        canonical,
                    }
                }
            }
            schema::Ty::Unknown { spelling } => Type::Unknown {
                spelling: spelling.clone(),
            },
        };
        self.program.types.intern(lowered)
    }

    fn lower_block(
        &mut self,
        stmt: &schema::Stmt,
        file: &str,
        nodes: &mut NodeIds,
    ) -> Result<Block, FrontendError> {
        let mut block = Block::default();
        self.lower_into(&mut block, stmt, file, nodes)?;
        Ok(block)
    }

    /// Lower `stmt` into `block`, splicing nested compound statements so the
    /// TIR block stays flat.
    fn lower_into(
        &mut self,
        block: &mut Block,
        stmt: &schema::Stmt,
        file: &str,
        nodes: &mut NodeIds,
    ) -> Result<(), FrontendError> {
        if let schema::Stmt::Compound { stmts } = stmt {
            for stmt in stmts {
                self.lower_into(block, stmt, file, nodes)?;
            }
            return Ok(());
        }
        if let Some(lowered) = self.lower_stmt(stmt, file, nodes)? {
            block.stmts.push(lowered);
        }
        Ok(())
    }

    /// Lower one statement. `None` means the statement has no TIR node: empty
    /// statements and bare compound blocks only contribute their children.
    fn lower_stmt(
        &mut self,
        stmt: &schema::Stmt,
        file: &str,
        nodes: &mut NodeIds,
    ) -> Result<Option<Stmt>, FrontendError> {
        let (id, kind, span) = match stmt {
            schema::Stmt::Compound { .. } => {
                return Err(FrontendError::new(
                    "<internal>",
                    "compound statements are spliced by the block lowerer",
                ))
            }
            schema::Stmt::Decl {
                name,
                ty,
                init,
                span,
            } => {
                let id = nodes.next();
                let ty = self.lower_type(ty);
                let init = match init {
                    Some(init) => Some(self.lower_expr(init, nodes)?),
                    None => None,
                };
                self.locals.insert(name.clone());
                (
                    id,
                    StmtKind::Decl {
                        name: name.clone(),
                        ty,
                        init,
                    },
                    span,
                )
            }
            schema::Stmt::Break { span } => (nodes.next(), StmtKind::Break, span),
            schema::Stmt::Continue { span } => (nodes.next(), StmtKind::Continue, span),
            schema::Stmt::DoWhile { body, cond, span } => {
                let id = nodes.next();
                let body = self.lower_block(body, file, nodes)?;
                let cond = self.lower_condition(cond, nodes)?;
                (id, StmtKind::DoWhile { body, cond }, span)
            }
            schema::Stmt::Switch {
                scrutinee,
                cases,
                span,
            } => {
                let id = nodes.next();
                let scrutinee = self.lower_expr(scrutinee, nodes)?;
                let mut lowered = Vec::with_capacity(cases.len());
                for case in cases {
                    let mut body = Block::default();
                    for child in &case.stmts {
                        self.lower_into(&mut body, child, file, nodes)?;
                    }
                    lowered.push(Case {
                        label: case.label,
                        body,
                        falls_through: case.falls_through,
                    });
                }
                (
                    id,
                    StmtKind::Switch {
                        scrutinee,
                        cases: lowered,
                    },
                    span,
                )
            }
            schema::Stmt::For {
                init,
                cond,
                step,
                body,
                span,
            } => {
                let id = nodes.next();
                let init = match init.as_deref() {
                    Some(init) => self.lower_stmt(init, file, nodes)?.map(Box::new),
                    None => None,
                };
                let cond = match cond {
                    Some(cond) => Some(self.lower_condition(cond, nodes)?),
                    None => None,
                };
                let step = match step {
                    Some(step) => Some(self.lower_expr(step, nodes)?),
                    None => None,
                };
                let body = self.lower_block(body, file, nodes)?;
                (
                    id,
                    StmtKind::For {
                        init,
                        cond,
                        step,
                        body,
                    },
                    span,
                )
            }
            schema::Stmt::While { cond, body, span } => {
                let id = nodes.next();
                let cond = self.lower_condition(cond, nodes)?;
                let body = self.lower_block(body, file, nodes)?;
                (id, StmtKind::While { cond, body }, span)
            }
            schema::Stmt::If {
                cond,
                then_branch,
                else_branch,
                span,
            } => {
                let id = nodes.next();
                let cond = self.lower_condition(cond, nodes)?;
                let then_branch = self.lower_block(then_branch, file, nodes)?;
                let else_branch = match else_branch.as_deref() {
                    Some(branch) => Some(self.lower_block(branch, file, nodes)?),
                    None => None,
                };
                (
                    id,
                    StmtKind::If {
                        cond,
                        then_branch,
                        else_branch,
                    },
                    span,
                )
            }
            schema::Stmt::Return { value, span } => {
                let id = nodes.next();
                let value = match value {
                    Some(value) => Some(self.lower_expr(value, nodes)?),
                    None => None,
                };
                (id, StmtKind::Return(value), span)
            }
            schema::Stmt::Expr { expr, span } => {
                let id = nodes.next();
                let expr = self.lower_expr(expr, nodes)?;
                (id, StmtKind::Expr(expr), span)
            }
            schema::Stmt::Empty => return Ok(None),
            schema::Stmt::Unsupported { spelling, span } => {
                return Err(FrontendError::new(
                    format!("{}:{}:{}", span.file, span.line, span.column),
                    format!("unsupported statement {spelling}"),
                ))
            }
        };
        Ok(Some(Stmt {
            id,
            kind,
            provenance: Provenance::source(span_of_raw(span, file)),
        }))
    }

    /// The size of an exported type under the target, for `sizeof` nodes that
    /// the exporter did not pre-resolve.
    fn schema_type_size(&self, ty: &schema::Ty) -> Option<u64> {
        match ty {
            schema::Ty::Bool => Some(1),
            schema::Ty::Int { bits, .. } => Some(u64::from(*bits) / 8),
            schema::Ty::Float { bits, .. } => Some(u64::from(*bits) / 8),
            schema::Ty::Pointer { .. } => Some(u64::from(self.program.target.pointer_bits) / 8),
            schema::Ty::Array { elem, len } => self.schema_type_size(elem).map(|size| size * len),
            schema::Ty::Record { name } => self.program.record(name).map(|record| record.size),
            schema::Ty::Enum { name } => self
                .program
                .enum_named(name)
                .and_then(|item| self.program.types.get(item.underlying).cloned())
                .and_then(|underlying| match underlying {
                    Type::Int { bits, .. } => Some(u64::from(bits) / 8),
                    _ => None,
                }),
            schema::Ty::Named { canonical, .. } => self.schema_type_size(canonical),
            schema::Ty::Void | schema::Ty::Unknown { .. } => None,
        }
    }

    /// Lower an initializer list, padding omitted elements with zero the way C
    /// does. Validation then sees a fully populated aggregate.
    fn lower_init_list(
        &mut self,
        target: TypeId,
        elems: &[schema::Expr],
        nodes: &mut NodeIds,
    ) -> Result<ExprKind, FrontendError> {
        let mut lowered = Vec::with_capacity(elems.len());
        for elem in elems {
            lowered.push(self.lower_expr(elem, nodes)?);
        }
        match self.program.types.get(target).cloned() {
            Some(Type::Array { elem, len }) => {
                if lowered.len() as u64 > len {
                    return Err(FrontendError::new(
                        "<initializer>",
                        format!(
                            "array initializer has {} elements but the array holds {len}",
                            lowered.len()
                        ),
                    ));
                }
                while (lowered.len() as u64) < len {
                    lowered.push(self.zero_literal(elem, nodes)?);
                }
            }
            Some(Type::Record { name }) => {
                let fields = self
                    .program
                    .record(&name)
                    .map(|record| {
                        record
                            .fields
                            .iter()
                            .map(|field| field.ty)
                            .collect::<Vec<_>>()
                    })
                    .ok_or_else(|| {
                        FrontendError::new("<initializer>", format!("record {name} is not defined"))
                    })?;
                if lowered.len() > fields.len() {
                    return Err(FrontendError::new(
                        "<initializer>",
                        format!(
                            "record {name} has {} fields but the initializer has {}",
                            fields.len(),
                            lowered.len()
                        ),
                    ));
                }
                for ty in fields.iter().skip(lowered.len()) {
                    lowered.push(self.zero_literal(*ty, nodes)?);
                }
            }
            _ => {
                return Err(FrontendError::new(
                    "<initializer>",
                    "an initializer list needs a record or array type",
                ))
            }
        }
        Ok(ExprKind::InitList(lowered))
    }

    fn zero_literal(&mut self, ty: TypeId, nodes: &mut NodeIds) -> Result<Expr, FrontendError> {
        let id = nodes.next();
        let kind = match self.program.types.canonical(ty) {
            Some(Type::Float { .. }) => ExprKind::FloatLit(0.0),
            Some(Type::Bool) => ExprKind::BoolLit(false),
            Some(Type::Int { .. }) | Some(Type::Char { .. }) | Some(Type::Enum { .. }) => {
                ExprKind::IntLit(0)
            }
            other => {
                return Err(FrontendError::new(
                    "<initializer>",
                    format!("cannot zero-initialize {other:?}"),
                ))
            }
        };
        Ok(Expr {
            id,
            ty,
            category: ValueCategory::PRValue,
            kind,
            provenance: Provenance::synthesized(),
        })
    }

    fn lower_condition(
        &mut self,
        cond: &schema::Expr,
        nodes: &mut NodeIds,
    ) -> Result<Expr, FrontendError> {
        let needs_cast = !matches!(cond.ty, schema::Ty::Bool);
        if !needs_cast {
            return self.lower_expr(cond, nodes);
        }
        // Conditions normalize to bool explicitly, which is what document 04
        // requires of every implicit conversion. The cast id is allocated
        // before the operand so ordinals stay in pre-order.
        let id = nodes.next();
        let to = self.program.types.intern(Type::Bool);
        let operand = self.lower_expr(cond, nodes)?;
        Ok(Expr {
            id,
            ty: to,
            category: ValueCategory::PRValue,
            kind: ExprKind::Cast {
                to,
                expr: Box::new(operand),
                implicit: true,
            },
            provenance: Provenance::synthesized(),
        })
    }

    fn lower_expr(
        &mut self,
        expr: &schema::Expr,
        nodes: &mut NodeIds,
    ) -> Result<Expr, FrontendError> {
        let id = nodes.next();
        let ty = self.lower_type(&expr.ty);
        // `recast-ast/1` records spans on statements and functions only, so
        // every expression is synthesized until the schema carries its span.
        let provenance = Provenance::synthesized();
        let expr = match &expr.kind {
            schema::ExprKind::IntLit { value } => Expr {
                id,
                ty,
                category: ValueCategory::PRValue,
                kind: ExprKind::IntLit(*value),
                provenance,
            },
            schema::ExprKind::FloatLit { value } => Expr {
                id,
                ty,
                category: ValueCategory::PRValue,
                kind: ExprKind::FloatLit(*value),
                provenance,
            },
            schema::ExprKind::StringLit { value } => Expr {
                id,
                ty,
                category: ValueCategory::LValue,
                kind: ExprKind::StringLit(value.clone()),
                provenance,
            },
            schema::ExprKind::CharLit { value } => Expr {
                id,
                ty,
                category: ValueCategory::PRValue,
                kind: ExprKind::CharLit(*value),
                provenance,
            },
            schema::ExprKind::DeclRef { name } => {
                let kind = if self.locals.contains(name) {
                    ExprKind::Local(name.clone())
                } else if let Some(item) = self.globals.get(name) {
                    ExprKind::Global(item.clone())
                } else if let Some((item, value)) = self.enumerators.get(name) {
                    ExprKind::EnumConst {
                        item: item.clone(),
                        value: *value,
                    }
                } else {
                    return Err(FrontendError::new(
                        format!(
                            "{}:{}:{}",
                            provenance.span.file, provenance.span.line, provenance.span.column
                        ),
                        format!("unresolved name {name}"),
                    ));
                };
                Expr {
                    id,
                    ty,
                    category: ValueCategory::LValue,
                    kind,
                    provenance,
                }
            }
            schema::ExprKind::Member { base, field, arrow } => {
                let base = self.lower_expr(base, nodes)?;
                Expr {
                    id,
                    ty,
                    category: ValueCategory::LValue,
                    kind: ExprKind::Member {
                        base: Box::new(base),
                        field: field.clone(),
                        arrow: *arrow,
                    },
                    provenance,
                }
            }
            schema::ExprKind::Cond {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond = self.lower_condition(cond, nodes)?;
                let then_branch = self.lower_expr(then_branch, nodes)?;
                let else_branch = self.lower_expr(else_branch, nodes)?;
                Expr {
                    id,
                    ty,
                    category: ValueCategory::PRValue,
                    kind: ExprKind::Cond {
                        cond: Box::new(cond),
                        then_branch: Box::new(then_branch),
                        else_branch: Box::new(else_branch),
                    },
                    provenance,
                }
            }
            schema::ExprKind::InitList { elems } => {
                let lowered = self.lower_init_list(ty, elems, nodes)?;
                Expr {
                    id,
                    ty,
                    category: ValueCategory::PRValue,
                    kind: lowered,
                    provenance,
                }
            }
            schema::ExprKind::SizeOf { operand } => {
                let size = self.schema_type_size(&operand.ty).ok_or_else(|| {
                    FrontendError::new(
                        "<sizeof>",
                        format!("cannot determine the size of {:?}", operand.ty),
                    )
                })?;
                Expr {
                    id,
                    ty,
                    category: ValueCategory::PRValue,
                    kind: ExprKind::IntLit(size as i64),
                    provenance,
                }
            }
            schema::ExprKind::Binary { op, lhs, rhs } => {
                let lhs = self.lower_expr(lhs, nodes)?;
                let rhs = self.lower_expr(rhs, nodes)?;
                Expr {
                    id,
                    ty,
                    category: ValueCategory::PRValue,
                    kind: ExprKind::Binary {
                        op: op.clone(),
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    provenance,
                }
            }
            schema::ExprKind::Assign { op, lhs, rhs } => {
                let lhs = self.lower_expr(lhs, nodes)?;
                let mut rhs = self.lower_expr(rhs, nodes)?;
                // `x += 1` computes in x's type; C's node carries the
                // unconverted literal, so the conversion is made explicit here.
                if self.program.types.canonical(lhs.ty) != self.program.types.canonical(rhs.ty) {
                    let cast_id = nodes.next();
                    rhs = Expr {
                        id: cast_id,
                        ty: lhs.ty,
                        category: ValueCategory::PRValue,
                        kind: ExprKind::Cast {
                            to: lhs.ty,
                            expr: Box::new(rhs),
                            implicit: true,
                        },
                        provenance: Provenance::synthesized(),
                    };
                }
                Expr {
                    id,
                    ty,
                    category: ValueCategory::PRValue,
                    kind: ExprKind::Assign {
                        op: op.clone(),
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    provenance,
                }
            }
            schema::ExprKind::Unary {
                op,
                operand,
                postfix,
            } => {
                let operand = self.lower_expr(operand, nodes)?;
                let category = if op == "*" {
                    ValueCategory::LValue
                } else {
                    ValueCategory::PRValue
                };
                Expr {
                    id,
                    ty,
                    category,
                    kind: ExprKind::Unary {
                        op: op.clone(),
                        operand: Box::new(operand),
                        postfix: *postfix,
                    },
                    provenance,
                }
            }
            schema::ExprKind::Index { base, index } => {
                let base = self.lower_expr(base, nodes)?;
                let index = self.lower_expr(index, nodes)?;
                Expr {
                    id,
                    ty,
                    category: ValueCategory::LValue,
                    kind: ExprKind::Index {
                        base: Box::new(base),
                        index: Box::new(index),
                    },
                    provenance,
                }
            }
            schema::ExprKind::Call { callee, args } => {
                let mut lowered = Vec::with_capacity(args.len());
                for arg in args {
                    lowered.push(self.lower_expr(arg, nodes)?);
                }
                let callee = match self.items.get(callee) {
                    Some(id) => Callee::Direct(id.clone()),
                    None => Callee::Unresolved(callee.clone()),
                };
                Expr {
                    id,
                    ty,
                    category: ValueCategory::PRValue,
                    kind: ExprKind::Call {
                        callee,
                        args: lowered,
                    },
                    provenance,
                }
            }
            schema::ExprKind::Cast { to, expr, implicit } => {
                let to = self.lower_type(to);
                let operand = self.lower_expr(expr, nodes)?;
                Expr {
                    id,
                    ty,
                    category: ValueCategory::PRValue,
                    kind: ExprKind::Cast {
                        to,
                        expr: Box::new(operand),
                        implicit: *implicit,
                    },
                    provenance,
                }
            }
            schema::ExprKind::Paren { expr } => {
                let inner = self.lower_expr(expr, nodes)?;
                let category = inner.category;
                Expr {
                    id,
                    ty,
                    category,
                    kind: ExprKind::Paren(Box::new(inner)),
                    provenance,
                }
            }
            schema::ExprKind::Unsupported { spelling } => {
                return Err(FrontendError::new(
                    format!(
                        "{}:{}:{}",
                        provenance.span.file, provenance.span.line, provenance.span.column
                    ),
                    format!("unsupported expression {spelling}"),
                ))
            }
        };
        Ok(expr)
    }
}

struct NodeIds {
    item: ItemId,
    next: u32,
}

impl NodeIds {
    fn new(item: ItemId) -> Self {
        Self { item, next: 0 }
    }

    fn next(&mut self) -> NodeId {
        let id = NodeId::new(self.item.clone(), self.next);
        self.next += 1;
        id
    }
}

fn span_of(function: &schema::Function, file: &str) -> Span {
    span_of_raw(&function.span, file)
}

fn span_of_raw(span: &schema::Span, file: &str) -> Span {
    // M0 lowers one translation unit per file, and the exporter reports the
    // span in that unit's path. Using the unit path keeps every span inside a
    // file the program has recorded.
    Span::new(file, span.line, span.column)
}

/// Strip type qualifiers so `const int` and `int` intern as one type.
///
/// Qualifiers on a pointee live in [`Type::Pointer::is_const`]; leaving them in
/// the spelling as well would give the same C type two TIR identities and
/// force spurious casts into translation.
fn normalize_spelling(spelling: &str) -> String {
    spelling
        .split_whitespace()
        .filter(|word| !matches!(*word, "const" | "volatile" | "restrict"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualifiers_do_not_create_new_integer_types() {
        assert_eq!(normalize_spelling("const int"), "int");
        assert_eq!(
            normalize_spelling("volatile unsigned long"),
            "unsigned long"
        );
        assert_eq!(normalize_spelling("signed char"), "signed char");
        assert_eq!(normalize_spelling("unsigned char"), "unsigned char");
    }
}
