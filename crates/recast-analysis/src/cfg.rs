//! The control-flow graph derived from a TIR function.
//!
//! The graph is rebuilt on demand and never serialized (document 04 section 5).
//! Blocks carry the local names they read and write so liveness and definite
//! assignment can run without re-walking the tree.

use std::collections::BTreeSet;

use recast_ir::{Block, Expr, ExprKind, Function, Stmt, StmtKind};

/// Position of a basic block in its graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub usize);

/// Where control goes after a block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Terminator {
    Goto(BlockId),
    Branch {
        then_block: BlockId,
        else_block: BlockId,
    },
    /// A `switch`: every case block plus the exit block.
    Switch(Vec<BlockId>),
    Return,
    /// Falling off the end of the function.
    #[default]
    Exit,
}

/// One basic block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BasicBlock {
    pub statements: Vec<recast_ir::NodeId>,
    pub uses: BTreeSet<String>,
    pub defs: BTreeSet<String>,
    pub terminator: Terminator,
}

/// A whole-function graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cfg {
    pub blocks: Vec<BasicBlock>,
    pub entry: BlockId,
}

impl Cfg {
    /// Successor blocks in a deterministic order.
    pub fn successors(&self, block: BlockId) -> Vec<BlockId> {
        match &self.blocks[block.0].terminator {
            Terminator::Goto(target) => vec![*target],
            Terminator::Branch {
                then_block,
                else_block,
            } => vec![*then_block, *else_block],
            Terminator::Switch(targets) => targets.clone(),
            Terminator::Return | Terminator::Exit => Vec::new(),
        }
    }

    /// Whether the block ends in a return.
    pub fn returns(&self, block: BlockId) -> bool {
        matches!(self.blocks[block.0].terminator, Terminator::Return)
    }
}

/// Build the graph for one function. Functions without a body have one empty
/// block.
pub fn build(function: &Function) -> Cfg {
    let mut builder = Builder::default();
    builder.current = builder.new_block();
    if let Some(body) = &function.body {
        builder.block(body);
    }
    Cfg {
        entry: BlockId(0),
        blocks: builder.blocks,
    }
}

#[derive(Default)]
struct Builder {
    blocks: Vec<BasicBlock>,
    current: usize,
    break_targets: Vec<usize>,
    continue_targets: Vec<usize>,
}

impl Builder {
    fn new_block(&mut self) -> usize {
        self.blocks.push(BasicBlock::default());
        self.blocks.len() - 1
    }

    fn set_current(&mut self, block: usize) {
        self.current = block;
    }

    fn block(&mut self, block: &Block) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        self.blocks[self.current].statements.push(stmt.id.clone());
        match &stmt.kind {
            StmtKind::Decl { name, init, .. } => {
                if let Some(init) = init {
                    self.expr(init);
                }
                self.blocks[self.current].defs.insert(name.clone());
            }
            StmtKind::Expr(expr) => self.expr(expr),
            StmtKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.expr(cond);
                let then_block = self.new_block();
                let else_block = self.new_block();
                let join = self.new_block();
                self.blocks[self.current].terminator = Terminator::Branch {
                    then_block: BlockId(then_block),
                    else_block: BlockId(else_block),
                };
                self.set_current(then_block);
                self.block(then_branch);
                self.blocks[self.current].terminator = Terminator::Goto(BlockId(join));
                self.set_current(else_block);
                if let Some(else_branch) = else_branch {
                    self.block(else_branch);
                }
                self.blocks[self.current].terminator = Terminator::Goto(BlockId(join));
                self.set_current(join);
            }
            StmtKind::While { cond, body } => {
                self.expr(cond);
                let header = self.current;
                let body_block = self.new_block();
                let exit = self.new_block();
                self.blocks[header].terminator = Terminator::Branch {
                    then_block: BlockId(body_block),
                    else_block: BlockId(exit),
                };
                self.break_targets.push(exit);
                self.continue_targets.push(header);
                self.set_current(body_block);
                self.block(body);
                self.blocks[self.current].terminator = Terminator::Goto(BlockId(header));
                self.break_targets.pop();
                self.continue_targets.pop();
                self.set_current(exit);
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some(init) = init {
                    self.stmt(init);
                }
                if let Some(cond) = cond {
                    self.expr(cond);
                }
                let header = self.current;
                let body_block = self.new_block();
                let step_block = self.new_block();
                let exit = self.new_block();
                self.blocks[header].terminator = Terminator::Branch {
                    then_block: BlockId(body_block),
                    else_block: BlockId(exit),
                };
                self.break_targets.push(exit);
                self.continue_targets.push(step_block);
                self.set_current(body_block);
                self.block(body);
                self.blocks[self.current].terminator = Terminator::Goto(BlockId(step_block));
                self.set_current(step_block);
                if let Some(step) = step {
                    self.expr(step);
                }
                self.blocks[self.current].terminator = Terminator::Goto(BlockId(header));
                self.break_targets.pop();
                self.continue_targets.pop();
                self.set_current(exit);
            }
            StmtKind::DoWhile { body, cond } => {
                let body_block = self.new_block();
                let cond_block = self.new_block();
                let exit = self.new_block();
                self.blocks[self.current].terminator = Terminator::Goto(BlockId(body_block));
                self.break_targets.push(exit);
                self.continue_targets.push(cond_block);
                self.set_current(body_block);
                self.block(body);
                self.blocks[self.current].terminator = Terminator::Goto(BlockId(cond_block));
                self.set_current(cond_block);
                self.expr(cond);
                self.blocks[self.current].terminator = Terminator::Branch {
                    then_block: BlockId(body_block),
                    else_block: BlockId(exit),
                };
                self.break_targets.pop();
                self.continue_targets.pop();
                self.set_current(exit);
            }
            StmtKind::Switch { scrutinee, cases } => {
                self.expr(scrutinee);
                let exit = self.new_block();
                let mut targets = Vec::with_capacity(cases.len());
                for _ in cases {
                    targets.push(self.new_block());
                }
                targets.push(exit);
                self.blocks[self.current].terminator =
                    Terminator::Switch(targets.iter().map(|id| BlockId(*id)).collect());
                self.break_targets.push(exit);
                for (index, case) in cases.iter().enumerate() {
                    self.set_current(targets[index]);
                    self.block(&case.body);
                    let next = if case.falls_through {
                        targets.get(index + 1).copied().unwrap_or(exit)
                    } else {
                        exit
                    };
                    self.blocks[self.current].terminator = Terminator::Goto(BlockId(next));
                }
                // The switch body itself may be empty between labels.
                self.set_current(targets[cases.len()]);
                self.break_targets.pop();
                self.set_current(exit);
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.expr(value);
                }
                self.blocks[self.current].terminator = Terminator::Return;
                let dead = self.new_block();
                self.set_current(dead);
            }
            StmtKind::Break => {
                if let Some(target) = self.break_targets.last() {
                    self.blocks[self.current].terminator = Terminator::Goto(BlockId(*target));
                    let dead = self.new_block();
                    self.set_current(dead);
                }
            }
            StmtKind::Continue => {
                if let Some(target) = self.continue_targets.last() {
                    self.blocks[self.current].terminator = Terminator::Goto(BlockId(*target));
                    let dead = self.new_block();
                    self.set_current(dead);
                }
            }
            StmtKind::Empty => {}
        }
    }

    fn expr(&mut self, expr: &Expr) {
        let block = &mut self.blocks[self.current];
        match &expr.kind {
            ExprKind::Local(name) => {
                block.uses.insert(name.clone());
            }
            ExprKind::Assign { op, lhs, rhs } => {
                collect_uses(lhs, &mut block.uses);
                collect_uses(rhs, &mut block.uses);
                if op == "=" {
                    if let Some(name) = root_local(lhs) {
                        block.defs.insert(name.to_string());
                    }
                } else if let Some(name) = root_local(lhs) {
                    block.defs.insert(name.to_string());
                    block.uses.insert(name.to_string());
                }
            }
            ExprKind::Unary { op, operand, .. } => {
                collect_uses(operand, &mut block.uses);
                if op == "++" || op == "--" {
                    if let Some(name) = root_local(operand) {
                        block.defs.insert(name.to_string());
                        block.uses.insert(name.to_string());
                    }
                }
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                collect_uses(lhs, &mut block.uses);
                collect_uses(rhs, &mut block.uses);
            }
            ExprKind::Index { base, index } => {
                collect_uses(base, &mut block.uses);
                collect_uses(index, &mut block.uses);
            }
            ExprKind::Member { base, .. } => collect_uses(base, &mut block.uses),
            ExprKind::Cond {
                cond,
                then_branch,
                else_branch,
            } => {
                collect_uses(cond, &mut block.uses);
                collect_uses(then_branch, &mut block.uses);
                collect_uses(else_branch, &mut block.uses);
            }
            ExprKind::InitList(elems) => {
                for elem in elems {
                    collect_uses(elem, &mut block.uses);
                }
            }
            ExprKind::Call { args, .. } => {
                for arg in args {
                    collect_uses(arg, &mut block.uses);
                }
            }
            ExprKind::Cast { expr, .. } | ExprKind::Paren(expr) => {
                collect_uses(expr, &mut block.uses)
            }
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
    }
}

pub(crate) fn root_local(expr: &Expr) -> Option<&str> {
    match &expr.kind {
        ExprKind::Local(name) => Some(name),
        ExprKind::Paren(inner) | ExprKind::Cast { expr: inner, .. } => root_local(inner),
        ExprKind::Member { base, .. } => root_local(base),
        ExprKind::Index { base, .. } => root_local(base),
        _ => None,
    }
}

pub(crate) fn collect_uses(expr: &Expr, uses: &mut BTreeSet<String>) {
    match &expr.kind {
        ExprKind::Local(name) => {
            uses.insert(name.clone());
        }
        ExprKind::Unary { operand, .. } => collect_uses(operand, uses),
        ExprKind::Binary { lhs, rhs, .. } | ExprKind::Assign { lhs, rhs, .. } => {
            collect_uses(lhs, uses);
            collect_uses(rhs, uses);
        }
        ExprKind::Index { base, index } => {
            collect_uses(base, uses);
            collect_uses(index, uses);
        }
        ExprKind::Member { base, .. } => collect_uses(base, uses),
        ExprKind::Cond {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_uses(cond, uses);
            collect_uses(then_branch, uses);
            collect_uses(else_branch, uses);
        }
        ExprKind::InitList(elems) => {
            for elem in elems {
                collect_uses(elem, uses);
            }
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                collect_uses(arg, uses);
            }
        }
        ExprKind::Cast { expr, .. } | ExprKind::Paren(expr) => collect_uses(expr, uses),
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
}
