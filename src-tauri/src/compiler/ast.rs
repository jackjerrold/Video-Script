//! Stage 2 output: the abstract syntax tree.
//!
//! This is a faithful, unchecked picture of what the user wrote. It knows
//! nothing about which shapes or properties exist: that is semantic analysis's
//! job. Keeping the AST generic (`rect box { ... }` is just `Ident Ident Block`)
//! is what makes new shapes and properties cheap to add.

use super::span::Span;
use super::value::Rgb;

#[derive(Debug, Clone)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Number(f64),
    Color(Rgb),
    Str(String),
    Ident(String),
    Unary { op: UnOp, expr: Box<Expr> },
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Call { callee: Ident, args: Vec<Expr> },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
        }
    }
}

/// `name: value [ease curve];`
#[derive(Debug, Clone)]
pub struct PropAssign {
    pub name: Ident,
    pub value: Expr,
    pub ease: Option<Ident>,
    pub span: Span,
}

/// `{ prop: value; ... }`
#[derive(Debug, Clone)]
pub struct Block {
    pub props: Vec<PropAssign>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub kind: ItemKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ItemKind {
    /// `video { ... }`
    Video(Block),
    /// `let name = expr;`
    Let { name: Ident, value: Expr },
    /// `rect box { ... }`: the kind is an identifier, resolved later.
    Shape { kind: Ident, name: Ident, body: Block },
    /// `at 2s box [ease curve] { ... }`
    At { time: Expr, target: Ident, ease: Option<Ident>, body: Block },
}

#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>,
}
