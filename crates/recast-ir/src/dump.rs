//! Deterministic textual dump used by golden tests and `recast analyze`.

use std::fmt::Write;

use crate::program::{
    Block, Callee, Expr, ExprKind, Function, Program, Stmt, StmtKind, SCHEMA_NAME,
};
use crate::types::TypeTable;

/// Render a whole program. The output depends only on the program.
pub fn dump(program: &Program) -> String {
    let mut out = String::new();
    let target = &program.target;
    writeln!(
        out,
        "{} schema={} target={} pointer={} int={} long={} long_long={} size_t={} endian={} char_signed={}",
        SCHEMA_NAME,
        program.schema_version,
        target.triple,
        target.pointer_bits,
        target.int_bits,
        target.long_bits,
        target.long_long_bits,
        target.size_t_bits,
        match target.endian {
            crate::program::Endian::Little => "little",
            crate::program::Endian::Big => "big",
        },
        target.char_is_signed,
    )
    .expect("writing to a String cannot fail");

    writeln!(out, "files:").expect("writing to a String cannot fail");
    for file in &program.files {
        writeln!(out, "  {}", file.path).expect("writing to a String cannot fail");
    }

    writeln!(out, "types:").expect("writing to a String cannot fail");
    for (id, _) in program.types.iter() {
        writeln!(out, "  {} = {}", id, program.types.render(id))
            .expect("writing to a String cannot fail");
    }

    for function in &program.functions {
        dump_function(function, &program.types, &mut out);
    }

    out
}

fn dump_function(function: &Function, types: &TypeTable, out: &mut String) {
    writeln!(
        out,
        "fn {} [{}] linkage={}",
        function.name,
        function.id,
        match function.linkage {
            crate::program::Linkage::External => "external",
            crate::program::Linkage::Internal => "internal",
            crate::program::Linkage::None => "none",
        }
    )
    .expect("writing to a String cannot fail");
    writeln!(out, "  ret {}", types.render(function.signature.ret))
        .expect("writing to a String cannot fail");
    writeln!(out, "  params").expect("writing to a String cannot fail");
    for param in &function.signature.params {
        match &param.name {
            Some(name) => writeln!(out, "    {name}: {}", types.render(param.ty)),
            None => writeln!(out, "    _: {}", types.render(param.ty)),
        }
        .expect("writing to a String cannot fail");
    }
    if function.signature.variadic {
        writeln!(out, "    ...").expect("writing to a String cannot fail");
    }
    match &function.body {
        Some(body) => {
            writeln!(out, "  body").expect("writing to a String cannot fail");
            dump_block(body, types, 2, out);
        }
        None => {
            writeln!(out, "  body <declaration>").expect("writing to a String cannot fail");
        }
    }
}

fn dump_block(block: &Block, types: &TypeTable, indent: usize, out: &mut String) {
    for stmt in &block.stmts {
        dump_stmt(stmt, types, indent, out);
    }
}

fn dump_stmt(stmt: &Stmt, types: &TypeTable, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    let node = stmt.id.ordinal();
    match &stmt.kind {
        StmtKind::Decl { name, ty, init } => {
            match init {
                Some(init) => writeln!(
                    out,
                    "{pad}@{node} decl {name}: {} = {}",
                    types.render(*ty),
                    render_expr(init, types)
                ),
                None => writeln!(out, "{pad}@{node} decl {name}: {}", types.render(*ty)),
            }
            .expect("writing to a String cannot fail");
        }
        StmtKind::Expr(expr) => {
            writeln!(out, "{pad}@{node} expr {}", render_expr(expr, types))
                .expect("writing to a String cannot fail");
        }
        StmtKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            writeln!(out, "{pad}@{node} if {}", render_expr(cond, types))
                .expect("writing to a String cannot fail");
            dump_block(then_branch, types, indent + 2, out);
            if let Some(else_branch) = else_branch {
                writeln!(out, "{pad}else").expect("writing to a String cannot fail");
                dump_block(else_branch, types, indent + 2, out);
            }
        }
        StmtKind::While { cond, body } => {
            writeln!(out, "{pad}@{node} while {}", render_expr(cond, types))
                .expect("writing to a String cannot fail");
            dump_block(body, types, indent + 2, out);
        }
        StmtKind::For {
            init,
            cond,
            step,
            body,
        } => {
            writeln!(out, "{pad}@{node} for").expect("writing to a String cannot fail");
            if let Some(init) = init {
                dump_stmt(init, types, indent + 2, out);
            }
            if let Some(cond) = cond {
                writeln!(out, "{pad}  cond {}", render_expr(cond, types))
                    .expect("writing to a String cannot fail");
            }
            if let Some(step) = step {
                writeln!(out, "{pad}  step {}", render_expr(step, types))
                    .expect("writing to a String cannot fail");
            }
            dump_block(body, types, indent + 2, out);
        }
        StmtKind::Return(value) => {
            match value {
                Some(value) => writeln!(out, "{pad}@{node} return {}", render_expr(value, types)),
                None => writeln!(out, "{pad}@{node} return"),
            }
            .expect("writing to a String cannot fail");
        }
        StmtKind::Break => {
            writeln!(out, "{pad}@{node} break").expect("writing to a String cannot fail");
        }
        StmtKind::Continue => {
            writeln!(out, "{pad}@{node} continue").expect("writing to a String cannot fail");
        }
        StmtKind::Empty => {
            writeln!(out, "{pad}@{node} empty").expect("writing to a String cannot fail");
        }
    }
}

fn render_expr(expr: &Expr, types: &TypeTable) -> String {
    let node = expr.id.ordinal();
    match &expr.kind {
        ExprKind::IntLit(value) => format!("@{node} int {value}"),
        ExprKind::FloatLit(value) => format!("@{node} float {value}"),
        ExprKind::StringLit(value) => format!("@{node} string {value:?}"),
        ExprKind::BoolLit(value) => format!("@{node} bool {value}"),
        ExprKind::Null => format!("@{node} null"),
        ExprKind::Local(name) => format!("@{node} local {name}"),
        ExprKind::Unary {
            op,
            operand,
            postfix,
        } => format!(
            "@{node} unary {}{} {}",
            if *postfix { "postfix " } else { "" },
            op,
            render_expr(operand, types)
        ),
        ExprKind::Binary { op, lhs, rhs } => format!(
            "@{node} binary {op} ({}) ({})",
            render_expr(lhs, types),
            render_expr(rhs, types)
        ),
        ExprKind::Assign { op, lhs, rhs } => format!(
            "@{node} assign {op} ({}) ({})",
            render_expr(lhs, types),
            render_expr(rhs, types)
        ),
        ExprKind::Index { base, index } => format!(
            "@{node} index ({}) ({})",
            render_expr(base, types),
            render_expr(index, types)
        ),
        ExprKind::Call { callee, args } => {
            let callee = match callee {
                Callee::Direct(id) => id.to_string(),
                Callee::Indirect(expr) => format!("({})", render_expr(expr, types)),
                Callee::Unresolved(name) => format!("<unresolved {name}>"),
            };
            let args = args
                .iter()
                .map(|arg| format!("({})", render_expr(arg, types)))
                .collect::<Vec<_>>()
                .join(" ");
            if args.is_empty() {
                format!("@{node} call {callee}")
            } else {
                format!("@{node} call {callee} {args}")
            }
        }
        ExprKind::Cast { to, expr, implicit } => format!(
            "@{node} cast {}{} ({})",
            if *implicit { "implicit " } else { "" },
            types.render(*to),
            render_expr(expr, types)
        ),
        ExprKind::Paren(expr) => format!("@{node} paren ({})", render_expr(expr, types)),
        ExprKind::Unsupported(spelling) => format!("@{node} unsupported {spelling}"),
    }
}
