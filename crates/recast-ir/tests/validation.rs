use recast_ir::{
    Block, Callee, Expr, ExprKind, FnSig, Function, ItemId, Linkage, NodeId, Param, Program,
    Provenance, Span, Stmt, StmtKind, Target, Type, TypeId, ValueCategory,
};

fn source() -> Provenance {
    Provenance::source(Span::new("add.c", 1, 1))
}

fn expr(item: &ItemId, ordinal: u32, ty: TypeId, kind: ExprKind) -> Expr {
    Expr {
        id: NodeId::new(item.clone(), ordinal),
        ty,
        category: ValueCategory::PRValue,
        kind,
        provenance: source(),
    }
}

fn stmt(item: &ItemId, ordinal: u32, kind: StmtKind) -> Stmt {
    Stmt {
        id: NodeId::new(item.clone(), ordinal),
        kind,
        provenance: source(),
    }
}

fn add_program() -> Program {
    let mut program = Program::new(Target::lp64("aarch64-unknown-linux-gnu"));
    program.add_file("add.c");
    let i32_ty = program.types.intern(Type::int(32, true, "int"));
    let item = ItemId::function("add");

    let lhs = expr(&item, 2, i32_ty, ExprKind::Local("a".to_string()));
    let rhs = expr(&item, 3, i32_ty, ExprKind::Local("b".to_string()));
    let sum = expr(
        &item,
        1,
        i32_ty,
        ExprKind::Binary {
            op: "+".to_string(),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
    );
    let body = Block {
        stmts: vec![stmt(&item, 0, StmtKind::Return(Some(sum)))],
    };

    program.add_function(Function {
        id: item,
        name: "add".to_string(),
        linkage: Linkage::External,
        signature: FnSig {
            params: vec![
                Param {
                    name: Some("a".to_string()),
                    ty: i32_ty,
                },
                Param {
                    name: Some("b".to_string()),
                    ty: i32_ty,
                },
            ],
            ret: i32_ty,
            variadic: false,
        },
        body: Some(body),
        provenance: source(),
    });
    program
}

#[test]
fn valid_program_passes_and_dumps_deterministically() {
    let program = add_program();
    program.validate().expect("the add program is valid");
    let first = program.dump();
    let second = program.dump();
    assert_eq!(first, second);
    assert!(first.contains("fn add [fn:c:@F@add] linkage=external"));
    assert!(first.contains("ret i32"));
    assert!(first.contains("binary +"));
}

#[test]
fn duplicate_items_are_rejected() {
    let mut program = add_program();
    let duplicate = program.functions[0].clone();
    program.add_function(duplicate);
    let errors = program.validate().expect_err("duplicate item must fail");
    assert!(errors.iter().any(|error| error.invariant == 1));
}

#[test]
fn unknown_types_are_rejected() {
    let mut program = add_program();
    let unknown = program.types.intern(Type::Unknown {
        spelling: "auto".to_string(),
    });
    program.functions[0].signature.ret = unknown;
    let errors = program.validate().expect_err("unknown type must fail");
    assert!(errors.iter().any(|error| error.invariant == 3));
}

#[test]
fn break_outside_a_loop_is_rejected() {
    let mut program = add_program();
    let item = ItemId::function("bad");
    let void_ty = program.types.intern(Type::Void);
    program.add_function(Function {
        id: item.clone(),
        name: "bad".to_string(),
        linkage: Linkage::External,
        signature: FnSig {
            params: Vec::new(),
            ret: void_ty,
            variadic: false,
        },
        body: Some(Block {
            stmts: vec![stmt(&item, 0, StmtKind::Break)],
        }),
        provenance: source(),
    });
    let errors = program.validate().expect_err("break must be rejected");
    assert!(errors.iter().any(|error| error.invariant == 5));
}

#[test]
fn direct_calls_need_the_right_arity() {
    let mut program = add_program();
    let caller = ItemId::function("caller");
    let i32_ty = program.types.intern(Type::int(32, true, "int"));
    let void_ty = program.types.intern(Type::Void);
    let callee = ItemId::function("add");
    let call = expr(
        &caller,
        0,
        i32_ty,
        ExprKind::Call {
            callee: Callee::Direct(callee),
            args: Vec::new(),
        },
    );
    program.add_function(Function {
        id: caller.clone(),
        name: "caller".to_string(),
        linkage: Linkage::External,
        signature: FnSig {
            params: Vec::new(),
            ret: void_ty,
            variadic: false,
        },
        body: Some(Block {
            stmts: vec![stmt(&caller, 1, StmtKind::Expr(call))],
        }),
        provenance: source(),
    });
    let errors = program.validate().expect_err("arity must be checked");
    assert!(errors.iter().any(|error| error.invariant == 4));
}

#[test]
fn mixed_operand_types_need_an_explicit_cast() {
    let mut program = add_program();
    let u32_ty = program.types.intern(Type::int(32, false, "unsigned int"));
    let body = program.functions[0].body.as_mut().expect("body");
    let StmtKind::Return(Some(returned)) = &mut body.stmts[0].kind else {
        panic!("expected return");
    };
    let ExprKind::Binary { rhs, .. } = &mut returned.kind else {
        panic!("expected binary");
    };
    rhs.ty = u32_ty;
    let errors = program.validate().expect_err("mixed operands must fail");
    assert!(errors.iter().any(|error| error.invariant == 7));
}
