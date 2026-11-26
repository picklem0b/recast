//! Analysis regression tests. Programs are built directly in TIR so the tests
//! do not depend on Clang.

use recast_analysis::analyze;
use recast_analysis::cfg::{self, BlockId};
use recast_ir::{
    Block, Expr, ExprKind, FnSig, Function, ItemId, Linkage, NodeId, Param, Program, Provenance,
    Span, Stmt, StmtKind, Target, Type, TypeId, ValueCategory,
};

struct Builder {
    program: Program,
    next: u32,
    item: ItemId,
}

impl Builder {
    fn new(name: &str) -> Self {
        Self {
            program: Program::new(Target::lp64("aarch64-unknown-linux-gnu")),
            next: 0,
            item: ItemId::function(name),
        }
    }

    fn int(&mut self) -> TypeId {
        self.program.types.intern(Type::int(32, true, "int"))
    }

    fn bool(&mut self) -> TypeId {
        self.program.types.intern(Type::Bool)
    }

    fn node(&mut self) -> NodeId {
        let node = NodeId::new(self.item.clone(), self.next);
        self.next += 1;
        node
    }

    fn local(&mut self, name: &str, ty: TypeId) -> Expr {
        let id = self.node();
        Expr {
            id,
            ty,
            category: ValueCategory::LValue,
            kind: ExprKind::Local(name.to_string()),
            provenance: Provenance::synthesized(),
        }
    }

    fn array(&mut self, elem: TypeId, len: u64) -> TypeId {
        self.program.types.intern(Type::Array { elem, len })
    }

    fn pointer(&mut self, pointee: TypeId, is_const: bool) -> TypeId {
        self.program
            .types
            .intern(Type::Pointer { pointee, is_const })
    }

    fn record(&mut self, name: &str) -> TypeId {
        self.program.types.intern(Type::Record {
            name: name.to_string(),
        })
    }

    fn member(&mut self, base: Expr, field: &str, arrow: bool, ty: TypeId) -> Expr {
        let id = self.node();
        Expr {
            id,
            ty,
            category: ValueCategory::LValue,
            kind: ExprKind::Member {
                base: Box::new(base),
                field: field.to_string(),
                arrow,
            },
            provenance: Provenance::synthesized(),
        }
    }

    fn index(&mut self, base: Expr, index: Expr, ty: TypeId) -> Expr {
        let id = self.node();
        Expr {
            id,
            ty,
            category: ValueCategory::LValue,
            kind: ExprKind::Index {
                base: Box::new(base),
                index: Box::new(index),
            },
            provenance: Provenance::synthesized(),
        }
    }

    fn cast(&mut self, expr: Expr, to: TypeId) -> Expr {
        let id = self.node();
        Expr {
            id,
            ty: to,
            category: ValueCategory::PRValue,
            kind: ExprKind::Cast {
                to,
                expr: Box::new(expr),
                implicit: true,
            },
            provenance: Provenance::synthesized(),
        }
    }

    fn int_lit(&mut self, value: i64, ty: TypeId) -> Expr {
        let id = self.node();
        Expr {
            id,
            ty,
            category: ValueCategory::PRValue,
            kind: ExprKind::IntLit(value),
            provenance: Provenance::synthesized(),
        }
    }

    fn binary(&mut self, op: &str, lhs: Expr, rhs: Expr, ty: TypeId) -> Expr {
        let id = self.node();
        Expr {
            id,
            ty,
            category: ValueCategory::PRValue,
            kind: ExprKind::Binary {
                op: op.to_string(),
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            provenance: Provenance::synthesized(),
        }
    }

    fn assign(&mut self, op: &str, lhs: Expr, rhs: Expr, ty: TypeId) -> Stmt {
        let id = self.node();
        let expr_id = self.node();
        Stmt {
            id,
            kind: StmtKind::Expr(Expr {
                id: expr_id,
                ty,
                category: ValueCategory::PRValue,
                kind: ExprKind::Assign {
                    op: op.to_string(),
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                provenance: Provenance::synthesized(),
            }),
            provenance: Provenance::synthesized(),
        }
    }

    fn return_stmt(&mut self, value: Expr) -> Stmt {
        let id = self.node();
        Stmt {
            id,
            kind: StmtKind::Return(Some(value)),
            provenance: Provenance::synthesized(),
        }
    }

    fn decl(&mut self, name: &str, ty: TypeId, init: Option<Expr>) -> Stmt {
        let id = self.node();
        Stmt {
            id,
            kind: StmtKind::Decl {
                name: name.to_string(),
                ty,
                init,
            },
            provenance: Provenance::synthesized(),
        }
    }

    fn finish(mut self, params: Vec<Param>, ret: TypeId, stmts: Vec<Stmt>) -> Program {
        let id = self.item.clone();
        let function = Function {
            id,
            name: "f".to_string(),
            linkage: Linkage::External,
            signature: FnSig {
                params,
                ret,
                variadic: false,
            },
            body: Some(Block { stmts }),
            provenance: Provenance::source(Span::new("t.c", 1, 1)),
        };
        self.program.add_file("t.c");
        self.program.add_function(function);
        self.program
    }
}

#[test]
fn division_by_zero_is_reported() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    let a = builder.local("a", int);
    let zero = builder.int_lit(0, int);
    let expr = builder.binary("/", a, zero, int);
    let body = builder.return_stmt(expr);
    let program = builder.finish(
        vec![Param {
            name: Some("a".to_string()),
            ty: int,
        }],
        int,
        vec![body],
    );
    let analysis = analyze(&program);
    assert_eq!(analysis.findings_with("UB-008").count(), 1);
}

#[test]
fn out_of_range_shift_is_reported() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    let a = builder.local("a", int);
    let count = builder.int_lit(40, int);
    let expr = builder.binary("<<", a, count, int);
    let body = builder.return_stmt(expr);
    let program = builder.finish(
        vec![Param {
            name: Some("a".to_string()),
            ty: int,
        }],
        int,
        vec![body],
    );
    let analysis = analyze(&program);
    assert_eq!(analysis.findings_with("UB-007").count(), 1);
}

#[test]
fn constant_signed_overflow_is_reported() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    let max = builder.int_lit(i64::from(i32::MAX), int);
    let one = builder.int_lit(1, int);
    let expr = builder.binary("+", max, one, int);
    let body = builder.return_stmt(expr);
    let program = builder.finish(Vec::new(), int, vec![body]);
    let analysis = analyze(&program);
    assert_eq!(analysis.findings_with("UB-001").count(), 1);
}

#[test]
fn reading_an_unassigned_local_is_reported() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    let decl = builder.decl("x", int, None);
    let read = builder.local("x", int);
    let body = builder.return_stmt(read);
    let program = builder.finish(Vec::new(), int, vec![decl, body]);
    let analysis = analyze(&program);
    assert_eq!(analysis.findings_with("UB-005").count(), 1);
}

#[test]
fn assignment_on_both_branches_satisfies_definite_assignment() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    let bool_ty = builder.bool();
    let decl = builder.decl("x", int, None);
    let cond = builder.local("c", bool_ty);

    let lhs = builder.local("x", int);
    let one = builder.int_lit(1, int);
    let then_assign = builder.assign("=", lhs, one, int);

    let lhs = builder.local("x", int);
    let two = builder.int_lit(2, int);
    let else_assign = builder.assign("=", lhs, two, int);

    let branch_id = builder.node();
    let branch = Stmt {
        id: branch_id,
        kind: StmtKind::If {
            cond,
            then_branch: Block {
                stmts: vec![then_assign],
            },
            else_branch: Some(Block {
                stmts: vec![else_assign],
            }),
        },
        provenance: Provenance::synthesized(),
    };
    let read = builder.local("x", int);
    let body = builder.return_stmt(read);
    let program = builder.finish(
        vec![Param {
            name: Some("c".to_string()),
            ty: bool_ty,
        }],
        int,
        vec![decl, branch, body],
    );
    let analysis = analyze(&program);
    assert_eq!(analysis.findings_with("UB-005").count(), 0);
}

#[test]
fn liveness_keeps_loop_carried_variables() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    // int total = 0; int i = 0; while (i < 3) { total += i; i += 1; } return total;
    let zero = builder.int_lit(0, int);
    let total = builder.decl("total", int, Some(zero));
    let zero = builder.int_lit(0, int);
    let i = builder.decl("i", int, Some(zero));

    let lhs = builder.local("i", int);
    let three = builder.int_lit(3, int);
    let cond = builder.binary("<", lhs, three, int);

    let lhs = builder.local("total", int);
    let rhs = builder.local("i", int);
    let add = builder.assign("+=", lhs, rhs, int);

    let operand = builder.local("i", int);
    let step_id = builder.node();
    let step_expr_id = builder.node();
    let step = Stmt {
        id: step_id,
        kind: StmtKind::Expr(Expr {
            id: step_expr_id,
            ty: int,
            category: ValueCategory::PRValue,
            kind: ExprKind::Unary {
                op: "++".to_string(),
                operand: Box::new(operand),
                postfix: true,
            },
            provenance: Provenance::synthesized(),
        }),
        provenance: Provenance::synthesized(),
    };

    let loop_id = builder.node();
    let loop_stmt = Stmt {
        id: loop_id,
        kind: StmtKind::While {
            cond,
            body: Block {
                stmts: vec![add, step],
            },
        },
        provenance: Provenance::synthesized(),
    };
    let read = builder.local("total", int);
    let body = builder.return_stmt(read);
    let program = builder.finish(Vec::new(), int, vec![total, i, loop_stmt, body]);
    let analysis = analyze(&program);
    let report = &analysis.functions[0];
    assert!(
        report.liveness.live_out(BlockId(0)).contains("total"),
        "total is read after its definition"
    );
    let graph = cfg::build(&program.functions[0]);
    assert!(graph.blocks.len() >= 4);
}

/// `struct point p; p.x = 1; return p.x;` stores into a field. The store does
/// not read the record, so it must not be reported as a read of `p`; the only
/// read of `p` is the one that follows the store, after the rule has marked it
/// assigned.
#[test]
fn storing_a_field_is_not_a_read_of_the_record() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    let point = builder.record("point");
    let decl = builder.decl("p", point, None);
    let base = builder.local("p", point);
    let field = builder.member(base, "x", false, int);
    let one = builder.int_lit(1, int);
    let store = builder.assign("=", field, one, int);
    let base = builder.local("p", point);
    let read = builder.member(base, "x", false, int);
    let body = builder.return_stmt(read);
    let program = builder.finish(Vec::new(), int, vec![decl, store, body]);
    let analysis = analyze(&program);
    assert_eq!(analysis.findings_with("UB-005").count(), 0);
}

/// `int a[3]; a[0] = 1; return a[0];`: the store targets an element, so the
/// array is not read by it.
#[test]
fn storing_an_array_element_is_not_a_read_of_the_array() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    let array = builder.array(int, 3);
    let decl = builder.decl("a", array, None);
    // The exporter decays the array with an implicit cast before indexing it.
    let pointer = builder.pointer(int, false);
    let source = builder.local("a", array);
    let base = builder.cast(source, pointer);
    let zero = builder.int_lit(0, int);
    let element = builder.index(base, zero, int);
    let one = builder.int_lit(1, int);
    let store = builder.assign("=", element, one, int);
    let source = builder.local("a", array);
    let base = builder.cast(source, pointer);
    let zero = builder.int_lit(0, int);
    let read = builder.index(base, zero, int);
    let body = builder.return_stmt(read);
    let program = builder.finish(Vec::new(), int, vec![decl, store, body]);
    let analysis = analyze(&program);
    assert_eq!(analysis.findings_with("UB-005").count(), 0);
}

/// `a[i] = 1` still reads the subscript, even though it does not read `a`.
#[test]
fn a_store_reads_its_subscript() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    let array = builder.array(int, 3);
    let array_decl = builder.decl("a", array, None);
    let index_decl = builder.decl("i", int, None);
    let pointer = builder.pointer(int, false);
    let source = builder.local("a", array);
    let base = builder.cast(source, pointer);
    let subscript = builder.local("i", int);
    let element = builder.index(base, subscript, int);
    let one = builder.int_lit(1, int);
    let store = builder.assign("=", element, one, int);
    let program = builder.finish(Vec::new(), int, vec![array_decl, index_decl, store]);
    let analysis = analyze(&program);
    assert_eq!(analysis.findings_with("UB-005").count(), 1);
}

/// A pointer base is a read: following an unassigned pointer is UB even when
/// the store itself only writes memory.
#[test]
fn storing_through_an_unassigned_pointer_is_reported() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    let pointer = builder.pointer(int, false);
    let decl = builder.decl("p", pointer, None);
    let source = builder.local("p", pointer);
    let base = builder.cast(source, pointer);
    let zero = builder.int_lit(0, int);
    let element = builder.index(base, zero, int);
    let one = builder.int_lit(1, int);
    let store = builder.assign("=", element, one, int);
    let program = builder.finish(Vec::new(), int, vec![decl, store]);
    let analysis = analyze(&program);
    assert_eq!(analysis.findings_with("UB-005").count(), 1);
}

/// `p->x = 1` reads the pointer to find the record, so an unassigned `p` is
/// still a finding.
#[test]
fn storing_a_field_through_an_unassigned_pointer_is_reported() {
    let mut builder = Builder::new("f");
    let int = builder.int();
    let point = builder.record("point");
    let pointer = builder.pointer(point, false);
    let decl = builder.decl("p", pointer, None);
    let base = builder.local("p", pointer);
    let field = builder.member(base, "x", true, int);
    let one = builder.int_lit(1, int);
    let store = builder.assign("=", field, one, int);
    let program = builder.finish(Vec::new(), int, vec![decl, store]);
    let analysis = analyze(&program);
    assert_eq!(analysis.findings_with("UB-005").count(), 1);
}
