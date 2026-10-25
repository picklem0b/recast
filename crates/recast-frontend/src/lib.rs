//! Lowering from the exporter schema to TIR.
//!
//! M0 accepts the L0 subset: scalar and pointer types, functions with a body,
//! declarations, assignments, calls, `if`, `while` and `for`. Anything outside
//! that subset is rejected by schema validation before it reaches this crate.
//!
//! Lowering is deterministic. Source files are sorted by path, functions by
//! name, and node identifiers are handed out in pre-order per function, so the
//! same export always produces the same program and the same dump.

use std::collections::BTreeMap;
use std::fmt;

use recast_ir::{
    Block, Callee, Endian, Expr, ExprKind, FnSig, Function, ItemId, Linkage, NodeId, Param,
    Program, Provenance, Span, Stmt, StmtKind, Target, Type, TypeId, ValueCategory,
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

    let mut items = BTreeMap::new();
    for (function, _) in &functions {
        items.insert(function.name.clone(), ItemId::function(&function.name));
    }

    let mut lowerer = Lowerer {
        program,
        items: items.clone(),
    };
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
}

impl Lowerer {
    fn lower_function(
        &mut self,
        function: &schema::Function,
        file: &str,
        id: &ItemId,
    ) -> Result<Function, FrontendError> {
        let ret = self.lower_type(&function.return_type);
        let mut params = Vec::with_capacity(function.params.len());
        for param in &function.params {
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
            schema::ExprKind::DeclRef { name } => Expr {
                id,
                ty,
                category: ValueCategory::LValue,
                kind: ExprKind::Local(name.clone()),
                provenance,
            },
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
                let rhs = self.lower_expr(rhs, nodes)?;
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
