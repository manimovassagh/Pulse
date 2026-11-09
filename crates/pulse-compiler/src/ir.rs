//! Typed intermediate representation.
//!
//! The IR is what the type checker produces and the optimizer and code
//! generator consume. Compared to the AST it is:
//!
//! * fully typed — every expression carries its [`Ty`];
//! * resolved — names are classified as locals, globals or functions, and
//!   built-ins (`signal`, `effect`, ...) have dedicated nodes;
//! * explicit about reactivity — reading `.value` is [`ExprKind::Read`],
//!   writing it is [`ExprKind::Write`], and string building is
//!   [`ExprKind::Concat`];
//! * DOM-shaped — JSX becomes a [`Node`] tree with attributes, event
//!   handlers and text separated.

use crate::ast::{AssignOp, BinOp, UnaryOp};
use std::fmt;

/// Pulse's static types, a subset of TypeScript's.
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Number,
    String,
    Bool,
    Void,
    /// `JSX.Element`: a DOM node.
    Element,
    Signal(Box<Ty>),
    Computed(Box<Ty>),
    Fn(Vec<Ty>, Box<Ty>),
}

impl Ty {
    /// Types that can be stored in a signal, compared and rendered as text.
    pub fn is_primitive(&self) -> bool {
        matches!(self, Ty::Number | Ty::String | Ty::Bool)
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Number => write!(f, "number"),
            Ty::String => write!(f, "string"),
            Ty::Bool => write!(f, "boolean"),
            Ty::Void => write!(f, "void"),
            Ty::Element => write!(f, "JSX.Element"),
            Ty::Signal(t) => write!(f, "Signal<{t}>"),
            Ty::Computed(t) => write!(f, "Computed<{t}>"),
            Ty::Fn(params, ret) => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "arg{i}: {p}")?;
                }
                write!(f, ") => {ret}")
            }
        }
    }
}

#[derive(Debug)]
pub struct Module {
    pub globals: Vec<Global>,
    pub functions: Vec<Function>,
    /// Name of the root component to mount.
    pub entry: String,
    /// Stylesheets imported with `import "./x.css"`, as written, in order.
    pub stylesheets: Vec<String>,
}

/// A module-level `const`/`let`, e.g. a shared signal.
#[derive(Debug)]
pub struct Global {
    pub name: String,
    pub ty: Ty,
    pub init: Expr,
}

/// A top-level function. Components are functions returning `Ty::Element`.
#[derive(Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let { name: String, init: Expr },
    Expr(Expr),
    Return(Option<Expr>),
    If { cond: Expr, then: Vec<Stmt>, els: Vec<Stmt> },
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Ty,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Num(f64),
    Str(String),
    Bool(bool),
    Local(String),
    Global(String),
    /// Tracked read of a signal or computed value (`x.value`).
    Read(Box<Expr>),
    /// Write to a signal (`x.value = v`, `x.value += v`, `x.value++`).
    Write { signal: Box<Expr>, op: AssignOp, value: Box<Expr> },
    Unary { op: UnaryOp, expr: Box<Expr> },
    /// Arithmetic, comparison and logical operators (never string `+`).
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    /// String concatenation; parts may be any primitive type and are
    /// converted to text with JavaScript semantics.
    Concat(Vec<Expr>),
    Cond { test: Box<Expr>, cons: Box<Expr>, alt: Box<Expr> },
    /// Call of a top-level function.
    Call { func: String, args: Vec<Expr> },
    /// Call of a function-typed value (a local or global closure).
    CallValue { callee: Box<Expr>, args: Vec<Expr> },
    Closure { params: Vec<(String, Ty)>, body: Vec<Stmt> },
    Signal(Box<Expr>),
    Computed(Box<Expr>),
    Effect(Box<Expr>),
    Batch(Box<Expr>),
    Untrack(Box<Expr>),
    Log(Vec<Expr>),
    Element(Box<Node>),
}

/// A DOM subtree produced from JSX.
#[derive(Debug, Clone)]
pub enum Node {
    Element { tag: String, attrs: Vec<Attr>, events: Vec<Event>, children: Vec<Node> },
    /// A text node; the expression is string-typed.
    Text(Expr),
    /// `<Child />` — a call of a component function.
    Component(String),
    /// `{expr}` where `expr` is a `JSX.Element` value.
    Embed(Expr),
}

/// An attribute; the value is `string` or `boolean` typed.
#[derive(Debug, Clone)]
pub struct Attr {
    pub name: String,
    pub value: Expr,
}

/// An event listener, e.g. `onClick` → `click`.
#[derive(Debug, Clone)]
pub struct Event {
    pub name: String,
    pub handler: Expr,
}

impl Expr {
    pub fn new(kind: ExprKind, ty: Ty) -> Self {
        Expr { kind, ty }
    }

    pub fn str(s: impl Into<String>) -> Self {
        Expr::new(ExprKind::Str(s.into()), Ty::String)
    }

    /// True if evaluating this expression may read reactive state, i.e. it
    /// must run inside an effect to stay up to date. Creating a closure is
    /// not a read; calling a function might be.
    pub fn is_reactive(&self) -> bool {
        match &self.kind {
            ExprKind::Num(_) | ExprKind::Str(_) | ExprKind::Bool(_) => false,
            ExprKind::Local(_) | ExprKind::Global(_) | ExprKind::Closure { .. } => false,
            ExprKind::Read(_) | ExprKind::Call { .. } | ExprKind::CallValue { .. } => true,
            ExprKind::Element(_) => true,
            ExprKind::Untrack(_) => false,
            ExprKind::Write { .. }
            | ExprKind::Signal(_)
            | ExprKind::Computed(_)
            | ExprKind::Effect(_)
            | ExprKind::Batch(_)
            | ExprKind::Log(_) => true,
            ExprKind::Unary { expr, .. } => expr.is_reactive(),
            ExprKind::Binary { left, right, .. } => left.is_reactive() || right.is_reactive(),
            ExprKind::Concat(parts) => parts.iter().any(Expr::is_reactive),
            ExprKind::Cond { test, cons, alt } => {
                test.is_reactive() || cons.is_reactive() || alt.is_reactive()
            }
        }
    }

    /// True for literal values.
    pub fn is_literal(&self) -> bool {
        matches!(self.kind, ExprKind::Num(_) | ExprKind::Str(_) | ExprKind::Bool(_))
    }
}
