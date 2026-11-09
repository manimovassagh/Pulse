//! IR optimizations.
//!
//! The passes are simple but matter for DOM performance:
//!
//! * **Constant folding** of arithmetic, comparisons, logic, conditionals and
//!   string concatenation (with JavaScript number formatting).
//! * **Text merging**: adjacent JSX text and `{expr}` children become a single
//!   DOM text node, so `Count: {n}` costs one node and one effect instead of
//!   two nodes.
//!
//! Whether a text node or attribute needs an effect at all is decided by the
//! code generator from [`Expr::is_reactive`]; after folding, constant content
//! is emitted as plain static DOM.

use crate::ast::{BinOp, UnaryOp};
use crate::ir::*;
use pulse_runtime::text::ToText;

pub fn optimize(module: &mut Module) {
    for g in &mut module.globals {
        fold(&mut g.init);
    }
    for f in &mut module.functions {
        stmts(&mut f.body);
    }
}

fn stmts(body: &mut [Stmt]) {
    for s in body {
        match s {
            Stmt::Let { init, .. } => fold(init),
            Stmt::Expr(e) => fold(e),
            Stmt::Return(Some(e)) => fold(e),
            Stmt::Return(None) => {}
            Stmt::If { cond, then, els } => {
                fold(cond);
                stmts(then);
                stmts(els);
            }
        }
    }
}

fn node(n: &mut Node) {
    match n {
        Node::Element { attrs, events, children, .. } => {
            for a in attrs {
                fold(&mut a.value);
            }
            for e in events {
                fold(&mut e.handler);
            }
            for c in children.iter_mut() {
                node(c);
            }
            merge_text(children);
        }
        Node::Text(e) | Node::Embed(e) => fold(e),
        Node::Component(_) => {}
    }
}

/// Merges runs of adjacent text children into one text node.
fn merge_text(children: &mut Vec<Node>) {
    let mut out: Vec<Node> = Vec::with_capacity(children.len());
    for child in children.drain(..) {
        match (out.last_mut(), child) {
            (Some(Node::Text(prev)), Node::Text(next)) => {
                let prev_expr = std::mem::replace(prev, Expr::str(""));
                let mut merged = Expr::new(ExprKind::Concat(vec![prev_expr, next]), Ty::String);
                fold(&mut merged);
                *prev = merged;
            }
            (_, Node::Text(e)) if matches!(&e.kind, ExprKind::Str(s) if s.is_empty()) => {}
            (_, child) => out.push(child),
        }
    }
    *children = out;
}

/// Folds constants in `e`, bottom-up.
pub fn fold(e: &mut Expr) {
    // Recurse first.
    match &mut e.kind {
        ExprKind::Read(x)
        | ExprKind::Signal(x)
        | ExprKind::Computed(x)
        | ExprKind::Effect(x)
        | ExprKind::Batch(x)
        | ExprKind::Untrack(x)
        | ExprKind::Unary { expr: x, .. } => fold(x),
        ExprKind::Write { signal, value, .. } => {
            fold(signal);
            fold(value);
        }
        ExprKind::Binary { left, right, .. } => {
            fold(left);
            fold(right);
        }
        ExprKind::Cond { test, cons, alt } => {
            fold(test);
            fold(cons);
            fold(alt);
        }
        ExprKind::Concat(parts) | ExprKind::Log(parts) | ExprKind::Call { args: parts, .. } => {
            parts.iter_mut().for_each(fold)
        }
        ExprKind::CallValue { callee, args } => {
            fold(callee);
            args.iter_mut().for_each(fold);
        }
        ExprKind::Closure { body, .. } => stmts(body),
        ExprKind::Element(n) => node(n),
        ExprKind::Num(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Local(_) | ExprKind::Global(_) => {}
    }

    let replacement = match &mut e.kind {
        ExprKind::Unary { op, expr } => match (op, &expr.kind) {
            (UnaryOp::Not, ExprKind::Bool(b)) => Some(ExprKind::Bool(!b)),
            (UnaryOp::Neg, ExprKind::Num(n)) => Some(ExprKind::Num(-n)),
            (UnaryOp::Plus, _) => Some(expr.kind.clone()),
            _ => None,
        },
        ExprKind::Binary { op, left, right } => fold_binary(*op, left, right),
        ExprKind::Cond { test, cons, alt } => match test.kind {
            ExprKind::Bool(true) => Some(cons.kind.clone()),
            ExprKind::Bool(false) => Some(alt.kind.clone()),
            _ => None,
        },
        ExprKind::Concat(parts) => Some(fold_concat(std::mem::take(parts))),
        _ => None,
    };
    if let Some(kind) = replacement {
        e.kind = kind;
    }
}

fn fold_binary(op: BinOp, l: &Expr, r: &Expr) -> Option<ExprKind> {
    use ExprKind::*;
    Some(match (&l.kind, &r.kind) {
        (Num(a), Num(b)) => match op {
            BinOp::Add => Num(a + b),
            BinOp::Sub => Num(a - b),
            BinOp::Mul => Num(a * b),
            BinOp::Div => Num(a / b),
            BinOp::Rem => Num(a % b),
            BinOp::Eq => Bool(a == b),
            BinOp::Ne => Bool(a != b),
            BinOp::Lt => Bool(a < b),
            BinOp::Le => Bool(a <= b),
            BinOp::Gt => Bool(a > b),
            BinOp::Ge => Bool(a >= b),
            BinOp::And | BinOp::Or => return None,
        },
        (Str(a), Str(b)) => match op {
            BinOp::Eq => Bool(a == b),
            BinOp::Ne => Bool(a != b),
            BinOp::Lt => Bool(a < b),
            BinOp::Le => Bool(a <= b),
            BinOp::Gt => Bool(a > b),
            BinOp::Ge => Bool(a >= b),
            _ => return None,
        },
        (Bool(a), Bool(b)) => match op {
            BinOp::Eq => Bool(a == b),
            BinOp::Ne => Bool(a != b),
            BinOp::And => Bool(*a && *b),
            BinOp::Or => Bool(*a || *b),
            _ => return None,
        },
        // Short-circuit identities that do not drop side effects.
        (Bool(false), _) if op == BinOp::And => Bool(false),
        (Bool(true), _) if op == BinOp::Or => Bool(true),
        (Bool(true), _) if op == BinOp::And => r.kind.clone(),
        (Bool(false), _) if op == BinOp::Or => r.kind.clone(),
        _ => return None,
    })
}

/// Flattens nested concatenations and joins adjacent literal parts.
fn fold_concat(parts: Vec<Expr>) -> ExprKind {
    let mut flat: Vec<Expr> = Vec::new();
    let mut stack: Vec<Expr> = parts.into_iter().rev().collect();
    while let Some(p) = stack.pop() {
        match p.kind {
            ExprKind::Concat(inner) => stack.extend(inner.into_iter().rev()),
            _ => flat.push(p),
        }
    }
    let mut out: Vec<Expr> = Vec::new();
    for p in flat {
        let text = literal_text(&p);
        match (out.last_mut(), text) {
            (Some(Expr { kind: ExprKind::Str(prev), .. }), Some(t)) => prev.push_str(&t),
            (_, Some(t)) if t.is_empty() => {}
            (_, Some(t)) => out.push(Expr::str(t)),
            (_, None) => out.push(p),
        }
    }
    match out.len() {
        0 => ExprKind::Str(String::new()),
        1 if out[0].ty == Ty::String => out.pop().unwrap().kind,
        _ => ExprKind::Concat(out),
    }
}

fn literal_text(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Str(s) => Some(s.clone()),
        ExprKind::Num(n) => Some(n.to_text()),
        ExprKind::Bool(b) => Some(b.to_text()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(src: &str) -> Module {
        let mut m = crate::lower::lower(&crate::parser::parse(src).unwrap()).unwrap();
        optimize(&mut m);
        m
    }

    fn global_init(m: &Module, name: &str) -> ExprKind {
        m.globals.iter().find(|g| g.name == name).unwrap().init.kind.clone()
    }

    fn root(m: &Module) -> Node {
        let f = m.functions.iter().find(|f| f.name == m.entry).unwrap();
        let Some(Stmt::Return(Some(Expr { kind: ExprKind::Element(n), .. }))) = f.body.last() else {
            panic!("no root element")
        };
        (**n).clone()
    }

    const APP: &str = "export default function App(): JSX.Element { return <div></div>; }\n";

    #[test]
    fn folds_arithmetic_and_strings() {
        let m = compile(&format!("const a = 1 + 2 * 3;\nconst b = \"v\" + (10 / 4) + true;\nconst c = 2 > 1 && !false;\n{APP}"));
        assert!(matches!(global_init(&m, "a"), ExprKind::Num(n) if n == 7.0));
        assert!(matches!(global_init(&m, "b"), ExprKind::Str(s) if s == "v2.5true"));
        assert!(matches!(global_init(&m, "c"), ExprKind::Bool(true)));
    }

    #[test]
    fn merges_static_and_dynamic_text_into_one_node() {
        let m = compile(
            "const n = signal(0);\nexport default function App(): JSX.Element { return <p>Count: {n.value} items{\"!\"}</p>; }",
        );
        let Node::Element { children, .. } = root(&m) else { panic!() };
        assert_eq!(children.len(), 1, "{children:#?}");
        let Node::Text(e) = &children[0] else { panic!() };
        let ExprKind::Concat(parts) = &e.kind else { panic!("{e:?}") };
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0].kind, ExprKind::Str(s) if s == "Count: "));
        assert!(matches!(&parts[2].kind, ExprKind::Str(s) if s == " items!"));
        assert!(e.is_reactive());
    }

    #[test]
    fn constant_expression_children_become_static_text() {
        let m = compile("export default function App(): JSX.Element { return <p>{1 + 1} is {\"two\"}</p>; }");
        let Node::Element { children, .. } = root(&m) else { panic!() };
        assert!(matches!(&children[..], [Node::Text(Expr { kind: ExprKind::Str(s), .. })] if s == "2 is two"));
    }
}
