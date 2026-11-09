//! Syntax tree for the supported TSX subset.

/// Byte range in the source file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start, other.end)
    }
}

#[derive(Debug)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug)]
pub enum Item {
    /// `import ... from "...";` — accepted and ignored (Pulse built-ins are
    /// always in scope).
    Import,
    Var(VarDecl),
    Function(FnDecl),
}

#[derive(Debug)]
pub struct VarDecl {
    pub name: String,
    pub name_span: Span,
    /// `let` rather than `const`.
    pub is_let: bool,
    pub ty: Option<TypeAnn>,
    pub init: Expr,
}

#[derive(Debug)]
pub struct Param {
    pub name: String,
    pub span: Span,
    pub ty: Option<TypeAnn>,
}

#[derive(Debug)]
pub struct FnDecl {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    pub ret: Option<TypeAnn>,
    pub body: Vec<Stmt>,
    pub is_default_export: bool,
}

/// A TypeScript type annotation.
#[derive(Debug, Clone)]
pub enum TypeAnn {
    /// `number`, `Signal<number>`, `JSX.Element`, ...
    Named { name: String, args: Vec<TypeAnn>, span: Span },
    /// `() => void`
    Fn { params: Vec<TypeAnn>, ret: Box<TypeAnn>, span: Span },
    /// Syntax we parse but do not support yet (arrays, unions, ...).
    Unsupported { span: Span },
}

impl TypeAnn {
    pub fn span(&self) -> Span {
        match self {
            TypeAnn::Named { span, .. } | TypeAnn::Fn { span, .. } | TypeAnn::Unsupported { span } => *span,
        }
    }
}

#[derive(Debug)]
pub enum Stmt {
    Var(VarDecl),
    Expr(Expr),
    Return(Option<Expr>, Span),
    If { cond: Expr, then: Vec<Stmt>, els: Option<Vec<Stmt>> },
}

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
    Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug)]
pub enum ExprKind {
    Num(f64),
    Str(String),
    Bool(bool),
    Ident(String),
    Member { obj: Box<Expr>, prop: String },
    Call { callee: Box<Expr>, type_args: Vec<TypeAnn>, args: Vec<Expr> },
    Unary { op: UnaryOp, expr: Box<Expr> },
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    Cond { test: Box<Expr>, cons: Box<Expr>, alt: Box<Expr> },
    Assign { op: AssignOp, target: Box<Expr>, value: Box<Expr> },
    /// `x++`, `--x`; `delta` is +1 or -1.
    Update { delta: f64, target: Box<Expr> },
    Arrow { params: Vec<Param>, ret: Option<TypeAnn>, body: ArrowBody },
    Jsx(JsxElement),
}

#[derive(Debug)]
pub enum ArrowBody {
    Expr(Box<Expr>),
    Block(Vec<Stmt>),
}

#[derive(Debug)]
pub struct JsxElement {
    /// Tag name; empty for a fragment `<>...</>`.
    pub name: String,
    pub attrs: Vec<JsxAttr>,
    pub children: Vec<JsxChild>,
    pub span: Span,
}

#[derive(Debug)]
pub struct JsxAttr {
    pub name: String,
    pub value: JsxAttrValue,
    pub span: Span,
}

#[derive(Debug)]
pub enum JsxAttrValue {
    /// `disabled` with no value.
    True,
    Str(String),
    Expr(Expr),
}

#[derive(Debug)]
pub enum JsxChild {
    Text(String, Span),
    Expr(Expr),
    Element(JsxElement),
}
