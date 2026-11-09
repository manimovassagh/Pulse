//! Name resolution and type checking: AST → typed IR.
//!
//! Pulse checks TypeScript types itself, because they decide the generated
//! Rust types (`number` → `f64`, `Signal<string>` → `Signal<String>`, ...).
//! The rules are deliberately a strict subset of TypeScript's: code that
//! Pulse accepts also type-checks under `tsc` with `pulse.d.ts`.
//!
//! Top-level declarations are resolved lazily, so they may be used before
//! they are declared (functions are hoisted, and globals are typically read
//! inside closures that run later).

use crate::ast::{self, ArrowBody, AssignOp, BinOp, JsxAttrValue, JsxChild, Span, TypeAnn, UnaryOp};
use crate::diagnostic::{Diagnostic, Result};
use crate::ir::*;
use std::collections::HashMap;

/// Type-checks a parsed module and lowers it to IR.
pub fn lower(program: &ast::Program) -> Result<Module> {
    let mut decls = HashMap::new();
    let mut tops = Vec::new();
    let mut stylesheets = Vec::new();
    for item in &program.items {
        let (name, span) = match item {
            ast::Item::Import(import) => {
                if let Some(css) = import_stylesheet(import)? {
                    stylesheets.push(css);
                }
                continue;
            }
            ast::Item::Var(v) => (&v.name, v.name_span),
            ast::Item::Function(f) => (&f.name, f.name_span),
        };
        if BUILTINS.contains(&name.as_str()) {
            return err(format!("`{name}` is a Pulse built-in and cannot be redeclared"), span);
        }
        if decls.insert(name.clone(), tops.len()).is_some() {
            return err(format!("`{name}` is declared more than once"), span);
        }
        tops.push(item);
    }

    let mut cx = Lowerer {
        decls,
        state: tops.iter().map(|_| State::Pending).collect(),
        out: tops.iter().map(|_| None).collect(),
        tops,
        scopes: Vec::new(),
        rets: Vec::new(),
    };
    for i in 0..cx.tops.len() {
        cx.resolve(i)?;
    }

    let entry = pick_entry(program)?;
    let mut globals = Vec::new();
    let mut functions = Vec::new();
    for out in cx.out.into_iter().flatten() {
        match out {
            Top::Global(g) => globals.push(g),
            Top::Function(f) => functions.push(f),
        }
    }
    let root = functions.iter().find(|f| f.name == entry).expect("entry resolved");
    if !root.params.is_empty() || root.ret != Ty::Element {
        let span = program
            .items
            .iter()
            .find_map(|i| match i {
                ast::Item::Function(f) if f.name == entry => Some(f.name_span),
                _ => None,
            })
            .unwrap_or_default();
        return err(format!("the root component `{entry}` must take no parameters and return JSX"), span);
    }
    Ok(Module { globals, functions, entry, stylesheets })
}

/// Validates an import. Returns the specifier if it is a stylesheet.
fn import_stylesheet(import: &ast::Import) -> Result<Option<String>> {
    let source = &import.source;
    if source == "pulse" {
        return Ok(None);
    }
    if source.ends_with(".css") {
        if !import.side_effect_only {
            return err(
                format!("CSS modules are not supported; import the file for its styles: `import \"{source}\";`"),
                import.source_span,
            );
        }
        return Ok(Some(source.clone()));
    }
    err(
        format!("cannot import `{source}`: only \"pulse\" and `.css` files can be imported for now"),
        import.source_span,
    )
}

/// The root component is the `export default function`, or else `App`.
fn pick_entry(program: &ast::Program) -> Result<String> {
    let functions = || {
        program.items.iter().filter_map(|i| match i {
            ast::Item::Function(f) => Some(f),
            _ => None,
        })
    };
    functions()
        .find(|f| f.is_default_export)
        .or_else(|| functions().find(|f| f.name == "App"))
        .map(|f| f.name.clone())
        .ok_or_else(|| {
            Diagnostic::new(
                "no root component: add `export default function App(): JSX.Element { ... }`",
                Span::default(),
            )
        })
}

const BUILTINS: &[&str] = &["signal", "computed", "effect", "batch", "untrack", "console"];

fn err<T>(msg: impl Into<String>, span: Span) -> Result<T> {
    Err(Diagnostic::new(msg, span))
}

fn expect_ty(expected: &Ty, found: &Ty, span: Span) -> Result<()> {
    if expected == found {
        Ok(())
    } else {
        err(format!("type mismatch: expected `{expected}`, found `{found}`"), span)
    }
}

enum State {
    Pending,
    InProgress,
    Done(Ty),
}

enum Top {
    Global(Global),
    Function(Function),
}

/// Return-type bookkeeping for the function or closure being lowered.
struct RetCtx {
    declared: Option<Ty>,
    inferred: Option<Ty>,
}

struct Lowerer<'a> {
    decls: HashMap<String, usize>,
    tops: Vec<&'a ast::Item>,
    state: Vec<State>,
    out: Vec<Option<Top>>,
    scopes: Vec<HashMap<String, Ty>>,
    rets: Vec<RetCtx>,
}

impl<'a> Lowerer<'a> {
    // ----- top-level declarations ----------------------------------------

    /// Resolves top-level declaration `i`, lowering it if needed, and returns
    /// its type.
    fn resolve(&mut self, i: usize) -> Result<Ty> {
        match &self.state[i] {
            State::Done(ty) => return Ok(ty.clone()),
            State::InProgress => {
                return match self.tops[i] {
                    ast::Item::Function(f) => err(
                        format!("`{}` is used recursively; add a return type annotation to it", f.name),
                        f.name_span,
                    ),
                    ast::Item::Var(v) => {
                        err(format!("`{}` is used in its own initializer", v.name), v.name_span)
                    }
                    ast::Item::Import(_) => unreachable!(),
                };
            }
            State::Pending => {}
        }
        self.state[i] = State::InProgress;
        // Lower in a clean context: the declaration cannot see the locals of
        // whatever code happened to trigger its resolution.
        let scopes = std::mem::take(&mut self.scopes);
        let rets = std::mem::take(&mut self.rets);
        let result = match self.tops[i] {
            ast::Item::Var(v) => self.global(v).map(|g| {
                let ty = g.ty.clone();
                (Top::Global(g), ty)
            }),
            ast::Item::Function(f) => self.function(i, f).map(|f| {
                let ty = Ty::Fn(f.params.iter().map(|p| p.1.clone()).collect(), Box::new(f.ret.clone()));
                (Top::Function(f), ty)
            }),
            ast::Item::Import(_) => unreachable!(),
        };
        self.scopes = scopes;
        self.rets = rets;
        let (top, ty) = result?;
        self.out[i] = Some(top);
        self.state[i] = State::Done(ty.clone());
        Ok(ty)
    }

    fn global(&mut self, v: &ast::VarDecl) -> Result<Global> {
        self.scopes.push(HashMap::new());
        let init = self.var_init(v)?;
        self.scopes.pop();
        Ok(Global { name: v.name.clone(), ty: init.ty.clone(), init })
    }

    fn function(&mut self, index: usize, f: &ast::FnDecl) -> Result<Function> {
        let mut params = Vec::new();
        let mut scope = HashMap::new();
        for p in &f.params {
            let Some(ann) = &p.ty else {
                return err(format!("parameter `{}` needs a type annotation", p.name), p.span);
            };
            let ty = self.resolve_type(ann)?;
            if scope.insert(p.name.clone(), ty.clone()).is_some() {
                return err(format!("duplicate parameter `{}`", p.name), p.span);
            }
            params.push((p.name.clone(), ty));
        }
        let declared = f.ret.as_ref().map(|t| self.resolve_type(t)).transpose()?;
        if let Some(ret) = &declared {
            // The signature is fully known: allow recursion.
            let sig = Ty::Fn(params.iter().map(|p| p.1.clone()).collect(), Box::new(ret.clone()));
            self.state[index] = State::Done(sig);
        }
        self.scopes.push(scope);
        self.rets.push(RetCtx { declared, inferred: None });
        let body = self.stmts(&f.body);
        self.scopes.pop();
        let ret_ctx = self.rets.pop().unwrap();
        let body = body?;
        let ret = ret_ctx.declared.or(ret_ctx.inferred).unwrap_or(Ty::Void);
        if ret != Ty::Void && !always_returns(&body) {
            return err(format!("`{}` must return a `{ret}` on every path", f.name), f.name_span);
        }
        Ok(Function { name: f.name.clone(), params, ret, body })
    }

    // ----- types ------------------------------------------------------------

    fn resolve_type(&mut self, ann: &TypeAnn) -> Result<Ty> {
        match ann {
            TypeAnn::Named { name, args, span } => {
                let arity = |n: usize| -> Result<()> {
                    if args.len() == n {
                        Ok(())
                    } else {
                        err(format!("`{name}` takes {n} type argument(s)"), *span)
                    }
                };
                match name.as_str() {
                    "number" => arity(0).map(|_| Ty::Number),
                    "string" => arity(0).map(|_| Ty::String),
                    "boolean" => arity(0).map(|_| Ty::Bool),
                    "void" => arity(0).map(|_| Ty::Void),
                    "JSX.Element" => arity(0).map(|_| Ty::Element),
                    "Signal" | "Computed" => {
                        arity(1)?;
                        let inner = self.resolve_type(&args[0])?;
                        if !inner.is_primitive() {
                            return err("signals can hold `number`, `string` or `boolean` values", args[0].span());
                        }
                        Ok(if name == "Signal" {
                            Ty::Signal(Box::new(inner))
                        } else {
                            Ty::Computed(Box::new(inner))
                        })
                    }
                    "any" | "unknown" => {
                        err(format!("`{name}` is not allowed in Pulse; use a concrete type"), *span)
                    }
                    _ => err(format!("unknown type `{name}`"), *span),
                }
            }
            TypeAnn::Fn { params, ret, .. } => {
                let params = params.iter().map(|p| self.resolve_type(p)).collect::<Result<_>>()?;
                Ok(Ty::Fn(params, Box::new(self.resolve_type(ret)?)))
            }
            TypeAnn::Unsupported { span } => err("this kind of type is not supported yet", *span),
        }
    }

    // ----- scopes -------------------------------------------------------------

    fn declare(&mut self, name: &str, ty: Ty, span: Span) -> Result<()> {
        if BUILTINS.contains(&name) {
            return err(format!("`{name}` is a Pulse built-in and cannot be redeclared"), span);
        }
        let scope = self.scopes.last_mut().expect("scope");
        if scope.insert(name.to_string(), ty).is_some() {
            return err(format!("`{name}` is already declared in this scope"), span);
        }
        Ok(())
    }

    fn lookup_local(&self, name: &str) -> Option<Ty> {
        self.scopes.iter().rev().find_map(|s| s.get(name).cloned())
    }

    fn is_builtin(&self, name: &str) -> bool {
        BUILTINS.contains(&name) && self.lookup_local(name).is_none() && !self.decls.contains_key(name)
    }

    // ----- statements -----------------------------------------------------------

    fn stmts(&mut self, stmts: &[ast::Stmt]) -> Result<Vec<Stmt>> {
        stmts.iter().map(|s| self.stmt(s)).collect()
    }

    fn block(&mut self, stmts: &[ast::Stmt]) -> Result<Vec<Stmt>> {
        self.scopes.push(HashMap::new());
        let out = self.stmts(stmts);
        self.scopes.pop();
        out
    }

    fn stmt(&mut self, stmt: &ast::Stmt) -> Result<Stmt> {
        Ok(match stmt {
            ast::Stmt::Var(v) => {
                let init = self.var_init(v)?;
                self.declare(&v.name, init.ty.clone(), v.name_span)?;
                Stmt::Let { name: v.name.clone(), init }
            }
            ast::Stmt::Expr(e) => Stmt::Expr(self.expr(e)?),
            ast::Stmt::Return(value, span) => {
                let value = value.as_ref().map(|e| self.expr(e)).transpose()?;
                let ty = value.as_ref().map_or(Ty::Void, |v| v.ty.clone());
                let at = value_span(stmt).unwrap_or(*span);
                let ctx = self.rets.last_mut().expect("return outside of a function");
                match (&ctx.declared, &ctx.inferred) {
                    (Some(expected), _) | (None, Some(expected)) => expect_ty(expected, &ty, at)?,
                    (None, None) => ctx.inferred = Some(ty),
                }
                Stmt::Return(value)
            }
            ast::Stmt::If { cond, then, els } => {
                let cond_ir = self.expr(cond)?;
                expect_ty(&Ty::Bool, &cond_ir.ty, cond.span)?;
                let then = self.block(then)?;
                let els = match els {
                    Some(e) => self.block(e)?,
                    None => Vec::new(),
                };
                Stmt::If { cond: cond_ir, then, els }
            }
        })
    }

    fn var_init(&mut self, v: &ast::VarDecl) -> Result<Expr> {
        let declared = v.ty.as_ref().map(|t| self.resolve_type(t)).transpose()?;
        let init = self.expr(&v.init)?;
        if init.ty == Ty::Void {
            return err("this expression has no value", v.init.span);
        }
        if let Some(declared) = declared {
            expect_ty(&declared, &init.ty, v.init.span)?;
        }
        Ok(init)
    }

    // ----- expressions ------------------------------------------------------------

    fn expr(&mut self, e: &ast::Expr) -> Result<Expr> {
        use ast::ExprKind as A;
        Ok(match &e.kind {
            A::Num(n) => Expr::new(ExprKind::Num(*n), Ty::Number),
            A::Str(s) => Expr::str(s.clone()),
            A::Bool(b) => Expr::new(ExprKind::Bool(*b), Ty::Bool),
            A::Ident(name) => self.ident(name, e.span)?,
            A::Member { obj, prop } => self.member(obj, prop, e.span)?,
            A::Call { callee, type_args, args } => self.call(callee, type_args, args, e.span)?,
            A::Unary { op, expr } => {
                let inner = self.expr(expr)?;
                let ty = match op {
                    UnaryOp::Not => Ty::Bool,
                    UnaryOp::Neg | UnaryOp::Plus => Ty::Number,
                };
                expect_ty(&ty, &inner.ty, expr.span)?;
                Expr::new(ExprKind::Unary { op: *op, expr: Box::new(inner) }, ty)
            }
            A::Binary { op, left, right } => self.binary(*op, left, right)?,
            A::Cond { test, cons, alt } => {
                let t = self.expr(test)?;
                expect_ty(&Ty::Bool, &t.ty, test.span)?;
                let c = self.expr(cons)?;
                let a = self.expr(alt)?;
                expect_ty(&c.ty, &a.ty, alt.span)?;
                let ty = c.ty.clone();
                Expr::new(ExprKind::Cond { test: Box::new(t), cons: Box::new(c), alt: Box::new(a) }, ty)
            }
            A::Assign { op, target, value } => {
                let signal = self.write_target(target)?;
                let inner = signal_inner(&signal.ty);
                let value_ir = self.expr(value)?;
                match op {
                    AssignOp::Assign => expect_ty(&inner, &value_ir.ty, value.span)?,
                    AssignOp::Add if inner == Ty::String => {
                        if !value_ir.ty.is_primitive() {
                            return err(format!("cannot append `{}` to a string", value_ir.ty), value.span);
                        }
                    }
                    _ => {
                        expect_ty(&Ty::Number, &inner, target.span)?;
                        expect_ty(&Ty::Number, &value_ir.ty, value.span)?;
                    }
                }
                Expr::new(ExprKind::Write { signal: Box::new(signal), op: *op, value: Box::new(value_ir) }, Ty::Void)
            }
            A::Update { delta, target } => {
                let signal = self.write_target(target)?;
                expect_ty(&Ty::Number, &signal_inner(&signal.ty), target.span)?;
                let one = Expr::new(ExprKind::Num(1.0), Ty::Number);
                let op = if *delta > 0.0 { AssignOp::Add } else { AssignOp::Sub };
                Expr::new(ExprKind::Write { signal: Box::new(signal), op, value: Box::new(one) }, Ty::Void)
            }
            A::Arrow { params, ret, body } => self.arrow(params, ret.as_ref(), body, e.span)?,
            A::Jsx(el) => {
                let node = self.jsx_root(el)?;
                Expr::new(ExprKind::Element(Box::new(node)), Ty::Element)
            }
        })
    }

    fn ident(&mut self, name: &str, span: Span) -> Result<Expr> {
        if let Some(ty) = self.lookup_local(name) {
            return Ok(Expr::new(ExprKind::Local(name.to_string()), ty));
        }
        if let Some(&i) = self.decls.get(name) {
            let ty = self.resolve(i)?;
            return Ok(match self.tops[i] {
                ast::Item::Var(_) => Expr::new(ExprKind::Global(name.to_string()), ty),
                _ => {
                    return err(
                        format!("functions are not values yet; call `{name}(...)` or wrap it in an arrow function"),
                        span,
                    )
                }
            });
        }
        if BUILTINS.contains(&name) {
            return err(format!("`{name}` must be called directly"), span);
        }
        err(format!("cannot find `{name}` in this scope"), span)
    }

    fn member(&mut self, obj: &ast::Expr, prop: &str, span: Span) -> Result<Expr> {
        let o = self.expr(obj)?;
        match (&o.ty, prop) {
            (Ty::Signal(inner) | Ty::Computed(inner), "value") => {
                let ty = (**inner).clone();
                Ok(Expr::new(ExprKind::Read(Box::new(o)), ty))
            }
            (ty, _) => err(format!("property `{prop}` does not exist on type `{ty}`"), span),
        }
    }

    /// Checks an assignment target: it must be `someSignal.value`.
    fn write_target(&mut self, target: &ast::Expr) -> Result<Expr> {
        if let ast::ExprKind::Member { obj, prop } = &target.kind {
            if prop == "value" {
                let o = self.expr(obj)?;
                return match o.ty {
                    Ty::Signal(_) => Ok(o),
                    Ty::Computed(_) => err("computed values are read-only", target.span),
                    ref other => err(format!("cannot assign to a property of `{other}`"), target.span),
                };
            }
        }
        if let ast::ExprKind::Ident(name) = &target.kind {
            return err(
                format!("cannot reassign `{name}`: Pulse variables are immutable; store changing state in a `signal`"),
                target.span,
            );
        }
        err("only `signal.value` can be assigned", target.span)
    }

    fn binary(&mut self, op: BinOp, left: &ast::Expr, right: &ast::Expr) -> Result<Expr> {
        let l = self.expr(left)?;
        let r = self.expr(right)?;
        let span = left.span.to(right.span);
        let ty = match op {
            BinOp::Add if l.ty == Ty::String || r.ty == Ty::String => {
                for (side, s) in [(&l, left.span), (&r, right.span)] {
                    if !side.ty.is_primitive() {
                        return err(format!("cannot concatenate `{}` to a string", side.ty), s);
                    }
                }
                return Ok(Expr::new(ExprKind::Concat(vec![l, r]), Ty::String));
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                expect_ty(&Ty::Number, &l.ty, left.span)?;
                expect_ty(&Ty::Number, &r.ty, right.span)?;
                Ty::Number
            }
            BinOp::Eq | BinOp::Ne => {
                if !l.ty.is_primitive() {
                    return err(format!("cannot compare values of type `{}`", l.ty), left.span);
                }
                if l.ty != r.ty {
                    return err(
                        format!("this comparison is always false: `{}` and `{}` have no overlap", l.ty, r.ty),
                        span,
                    );
                }
                Ty::Bool
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                if !matches!(l.ty, Ty::Number | Ty::String) {
                    return err(format!("cannot order values of type `{}`", l.ty), left.span);
                }
                expect_ty(&l.ty, &r.ty, right.span)?;
                Ty::Bool
            }
            BinOp::And | BinOp::Or => {
                let msg = |ty: &Ty| format!("`&&` and `||` need `boolean` operands, found `{ty}`");
                if l.ty != Ty::Bool {
                    return err(msg(&l.ty), left.span);
                }
                if r.ty != Ty::Bool {
                    return err(msg(&r.ty), right.span);
                }
                Ty::Bool
            }
        };
        Ok(Expr::new(ExprKind::Binary { op, left: Box::new(l), right: Box::new(r) }, ty))
    }

    fn arrow(&mut self, params: &[ast::Param], ret: Option<&TypeAnn>, body: &ArrowBody, span: Span) -> Result<Expr> {
        let mut scope = HashMap::new();
        let mut ir_params = Vec::new();
        for p in params {
            let Some(ann) = &p.ty else {
                return err(
                    format!("parameter `{}` needs a type annotation (event objects are not supported yet)", p.name),
                    p.span,
                );
            };
            let ty = self.resolve_type(ann)?;
            scope.insert(p.name.clone(), ty.clone());
            ir_params.push((p.name.clone(), ty));
        }
        let declared = ret.map(|t| self.resolve_type(t)).transpose()?;
        self.scopes.push(scope);
        self.rets.push(RetCtx { declared: declared.clone(), inferred: None });
        let result = match body {
            ArrowBody::Expr(e) => self.expr(e).and_then(|v| {
                if let Some(d) = &declared {
                    if *d != Ty::Void {
                        expect_ty(d, &v.ty, e.span)?;
                    }
                }
                let ret_ty = declared.clone().unwrap_or_else(|| v.ty.clone());
                let stmt = if ret_ty == Ty::Void { Stmt::Expr(v) } else { Stmt::Return(Some(v)) };
                Ok((vec![stmt], ret_ty))
            }),
            ArrowBody::Block(stmts) => self.stmts(stmts).map(|b| {
                let ctx = self.rets.last().unwrap();
                let ret_ty = ctx.declared.clone().or(ctx.inferred.clone()).unwrap_or(Ty::Void);
                (b, ret_ty)
            }),
        };
        self.scopes.pop();
        self.rets.pop();
        let (body, ret_ty) = result?;
        if ret_ty != Ty::Void && !always_returns(&body) {
            return err(format!("arrow function must return a `{ret_ty}` on every path"), span);
        }
        let ty = Ty::Fn(ir_params.iter().map(|p| p.1.clone()).collect(), Box::new(ret_ty));
        Ok(Expr::new(ExprKind::Closure { params: ir_params, body }, ty))
    }

    fn call(&mut self, callee: &ast::Expr, type_args: &[TypeAnn], args: &[ast::Expr], span: Span) -> Result<Expr> {
        // Built-ins.
        if let ast::ExprKind::Ident(name) = &callee.kind {
            if self.is_builtin(name) {
                return self.builtin(name, type_args, args, span);
            }
        }
        if let ast::ExprKind::Member { obj, prop } = &callee.kind {
            if matches!(&obj.kind, ast::ExprKind::Ident(c) if c == "console" && self.is_builtin("console")) {
                if prop != "log" {
                    return err(format!("`console.{prop}` is not supported yet; use `console.log`"), callee.span);
                }
                let mut parts = Vec::new();
                for a in args {
                    let v = self.expr(a)?;
                    if !v.ty.is_primitive() {
                        return err(format!("cannot log a value of type `{}`", v.ty), a.span);
                    }
                    parts.push(v);
                }
                return Ok(Expr::new(ExprKind::Log(parts), Ty::Void));
            }
        }
        if !type_args.is_empty() {
            return err("type arguments are only supported on Pulse built-ins", span);
        }
        // Top-level functions are called directly; anything else must be a
        // function-typed value.
        let (target, sig) = match &callee.kind {
            ast::ExprKind::Ident(name) if self.lookup_local(name).is_none() => match self.decls.get(name) {
                Some(&i) if matches!(self.tops[i], ast::Item::Function(_)) => (Err(name.clone()), self.resolve(i)?),
                _ => {
                    let v = self.expr(callee)?;
                    let ty = v.ty.clone();
                    (Ok(v), ty)
                }
            },
            _ => {
                let v = self.expr(callee)?;
                let ty = v.ty.clone();
                (Ok(v), ty)
            }
        };
        let Ty::Fn(params, ret) = sig else {
            return err(format!("type `{sig}` is not callable"), callee.span);
        };
        if params.len() != args.len() {
            return err(format!("expected {} argument(s), found {}", params.len(), args.len()), span);
        }
        let mut ir_args = Vec::new();
        for (p, a) in params.iter().zip(args) {
            let v = self.expr(a)?;
            expect_ty(p, &v.ty, a.span)?;
            ir_args.push(v);
        }
        let kind = match target {
            Err(func) => ExprKind::Call { func, args: ir_args },
            Ok(callee) => ExprKind::CallValue { callee: Box::new(callee), args: ir_args },
        };
        Ok(Expr::new(kind, *ret))
    }

    fn builtin(&mut self, name: &str, type_args: &[TypeAnn], args: &[ast::Expr], span: Span) -> Result<Expr> {
        if args.len() != 1 {
            return err(format!("`{name}` takes exactly one argument"), span);
        }
        let annotated = match type_args {
            [] => None,
            [t] if matches!(name, "signal" | "computed") => Some(self.resolve_type(t)?),
            _ => return err(format!("unexpected type arguments for `{name}`"), span),
        };
        let arg = &args[0];
        let value = self.expr(arg)?;
        match name {
            "signal" => {
                let ty = annotated.unwrap_or_else(|| value.ty.clone());
                expect_ty(&ty, &value.ty, arg.span)?;
                if !ty.is_primitive() {
                    return err("signals can hold `number`, `string` or `boolean` values", arg.span);
                }
                Ok(Expr::new(ExprKind::Signal(Box::new(value)), Ty::Signal(Box::new(ty))))
            }
            _ => {
                let Ty::Fn(params, ret) = &value.ty else {
                    return err(format!("`{name}` expects a function, e.g. `{name}(() => ...)`"), arg.span);
                };
                if !params.is_empty() {
                    return err(format!("the function passed to `{name}` must not take parameters"), arg.span);
                }
                let ret = (**ret).clone();
                match name {
                    "computed" => {
                        let ty = annotated.unwrap_or_else(|| ret.clone());
                        expect_ty(&ty, &ret, arg.span)?;
                        if !ty.is_primitive() {
                            return err("computed values must be `number`, `string` or `boolean`", arg.span);
                        }
                        Ok(Expr::new(ExprKind::Computed(Box::new(value)), Ty::Computed(Box::new(ty))))
                    }
                    "effect" => Ok(Expr::new(ExprKind::Effect(Box::new(value)), Ty::Void)),
                    "batch" => Ok(Expr::new(ExprKind::Batch(Box::new(value)), ret)),
                    "untrack" => Ok(Expr::new(ExprKind::Untrack(Box::new(value)), ret)),
                    _ => unreachable!(),
                }
            }
        }
    }

    // ----- JSX --------------------------------------------------------------------

    fn jsx_root(&mut self, el: &ast::JsxElement) -> Result<Node> {
        if el.name.is_empty() {
            return err(
                "fragments are only supported inside an element for now; wrap the content in an element",
                el.span,
            );
        }
        self.jsx(el)
    }

    fn jsx(&mut self, el: &ast::JsxElement) -> Result<Node> {
        let first = el.name.chars().next().unwrap_or('a');
        if first.is_uppercase() {
            return self.jsx_component(el);
        }
        let mut attrs = Vec::new();
        let mut events = Vec::new();
        for a in &el.attrs {
            if a.name == "key" {
                continue;
            }
            if let Some(event) = event_name(&a.name) {
                let JsxAttrValue::Expr(e) = &a.value else {
                    return err(format!("`{}` expects a function, e.g. `{}={{() => ...}}`", a.name, a.name), a.span);
                };
                let handler = self.expr(e)?;
                if !matches!(&handler.ty, Ty::Fn(p, _) if p.is_empty()) {
                    return err(
                        format!("event handlers must be functions without parameters, found `{}`", handler.ty),
                        e.span,
                    );
                }
                events.push(Event { name: event, handler });
                continue;
            }
            let name = match a.name.as_str() {
                "className" => "class".to_string(),
                "htmlFor" => "for".to_string(),
                other => other.to_string(),
            };
            let value = match &a.value {
                JsxAttrValue::True => Expr::new(ExprKind::Bool(true), Ty::Bool),
                JsxAttrValue::Str(s) => Expr::str(s.clone()),
                JsxAttrValue::Expr(e) => {
                    let v = self.expr(e)?;
                    match v.ty {
                        Ty::String | Ty::Bool => v,
                        Ty::Number => Expr::new(ExprKind::Concat(vec![v]), Ty::String),
                        ref other => {
                            return err(format!("attribute `{name}` cannot be set to a `{other}`"), e.span)
                        }
                    }
                }
            };
            attrs.push(Attr { name, value });
        }
        let mut children = Vec::new();
        self.jsx_children(&el.children, &mut children)?;
        Ok(Node::Element { tag: el.name.clone(), attrs, events, children })
    }

    fn jsx_children(&mut self, kids: &[JsxChild], out: &mut Vec<Node>) -> Result<()> {
        for child in kids {
            match child {
                JsxChild::Text(t, _) => out.push(Node::Text(Expr::str(t.clone()))),
                JsxChild::Element(el) if el.name.is_empty() => self.jsx_children(&el.children, out)?,
                JsxChild::Element(el) => out.push(self.jsx(el)?),
                JsxChild::Expr(e) => {
                    let v = self.expr(e)?;
                    match v.ty {
                        Ty::Number | Ty::String => out.push(Node::Text(Expr::new(ExprKind::Concat(vec![v]), Ty::String))),
                        Ty::Element => {
                            if v.is_reactive() && !matches!(v.kind, ExprKind::Element(_)) {
                                return err(
                                    "reactive conditional rendering is not supported yet (planned for milestone 2)",
                                    e.span,
                                );
                            }
                            out.push(Node::Embed(v))
                        }
                        Ty::Bool => {
                            return err("booleans are not rendered; convert it to a string first", e.span)
                        }
                        ref other => return err(format!("a `{other}` cannot be rendered"), e.span),
                    }
                }
            }
        }
        Ok(())
    }

    fn jsx_component(&mut self, el: &ast::JsxElement) -> Result<Node> {
        let span = el.span;
        let sig = match self.decls.get(&el.name) {
            Some(&i) if matches!(self.tops[i], ast::Item::Function(_)) => self.resolve(i)?,
            _ => return err(format!("cannot find component `{}`", el.name), span),
        };
        if !matches!(&sig, Ty::Fn(p, r) if p.is_empty() && **r == Ty::Element) {
            return err(format!("`{}` is not a component (found `{sig}`)", el.name), span);
        }
        if !el.attrs.is_empty() {
            return err("component props are not supported yet (planned for milestone 2)", el.attrs[0].span);
        }
        if !el.children.is_empty() {
            return err("passing children to components is not supported yet", span);
        }
        Ok(Node::Component(el.name.clone()))
    }
}

/// `onClick` → `click`, `onMouseEnter` → `mouseenter`.
fn event_name(attr: &str) -> Option<String> {
    let rest = attr.strip_prefix("on")?;
    rest.starts_with(|c: char| c.is_ascii_uppercase()).then(|| rest.to_ascii_lowercase())
}

fn signal_inner(ty: &Ty) -> Ty {
    match ty {
        Ty::Signal(t) | Ty::Computed(t) => (**t).clone(),
        other => other.clone(),
    }
}

fn value_span(stmt: &ast::Stmt) -> Option<Span> {
    match stmt {
        ast::Stmt::Return(Some(e), _) => Some(e.span),
        _ => None,
    }
}

/// True if executing `body` always reaches a `return`.
fn always_returns(body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::Return(_) => true,
        Stmt::If { then, els, .. } => always_returns(then) && always_returns(els),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(src: &str) -> Result<Module> {
        lower(&crate::parser::parse(src).unwrap())
    }

    /// Compiles `body` inside an `App` component and returns the error message.
    fn error_in_app(body: &str) -> String {
        let src = format!("export default function App(): JSX.Element {{\n{body}\nreturn <div></div>;\n}}");
        match check(&src) {
            Ok(_) => panic!("expected an error for:\n{body}"),
            Err(e) => e.message,
        }
    }

    #[test]
    fn accepts_valid_program() {
        let m = check(
            r#"
            const count = signal<number>(0);
            const doubled: Computed<number> = computed(() => count.value * 2);
            function label(n: number): string { return "n=" + n; }
            function Child(): JSX.Element { return <span>{label(doubled.value)}</span>; }
            export default function App(): JSX.Element {
              const inc: () => void = () => count.value++;
              return <div onClick={inc}><Child /></div>;
            }
            "#,
        )
        .unwrap();
        assert_eq!(m.entry, "App");
        assert_eq!(m.globals[0].ty, Ty::Signal(Box::new(Ty::Number)));
        assert_eq!(m.functions.len(), 3);
    }

    #[test]
    fn type_errors() {
        let cases = [
            (r#"const s = signal<number>("zero");"#, "expected `number`, found `string`"),
            (r#"const n: string = 5;"#, "expected `string`, found `number`"),
            ("const s = signal(0); s.value = true;", "expected `number`, found `boolean`"),
            ("const c = computed(() => 1); c.value = 2;", "computed values are read-only"),
            ("const x = 1; x = 2;", "Pulse variables are immutable"),
            ("const b = 1 && true;", "need `boolean` operands"),
            (r#"const b = 1 === "1";"#, "always false"),
            ("const y = missing + 1;", "cannot find `missing`"),
            ("const s = signal(\"a\"); s.value++;", "expected `number`, found `string`"),
            ("const s = signal(0); const z = s.length;", "property `length` does not exist"),
            ("const f = (x) => x;", "needs a type annotation"),
            ("const a: any = 1;", "`any` is not allowed"),
            ("const v = console.log(1);", "has no value"),
        ];
        for (code, expected) in cases {
            let msg = error_in_app(code);
            assert!(msg.contains(expected), "for `{code}`:\n  expected error containing {expected:?}\n  got {msg:?}");
        }
    }

    #[test]
    fn jsx_errors() {
        let cases = [
            ("<div onClick={1}></div>", "event handlers must be functions"),
            ("<div onClick=\"x\"></div>", "expects a function"),
            ("<div>{true}</div>", "booleans are not rendered"),
            ("<Nope />", "cannot find component `Nope`"),
            ("<Child title=\"x\" />", "props are not supported yet"),
            ("<div>{signal(1).value > 0 ? <a></a> : <b></b>}</div>", "reactive conditional rendering"),
        ];
        for (jsx, expected) in cases {
            let src = format!(
                "function Child(): JSX.Element {{ return <i></i>; }}\nexport default function App(): JSX.Element {{ return {jsx}; }}"
            );
            let msg = check(&src).map(|_| String::new()).unwrap_or_else(|e| e.message);
            assert!(msg.contains(expected), "for `{jsx}`: got {msg:?}");
        }
    }

    #[test]
    fn functions_must_return_on_every_path() {
        let msg = check(
            "function f(n: number): number { if (n > 0) { return 1; } }\nexport default function App(): JSX.Element { return <p></p>; }",
        )
        .unwrap_err()
        .message;
        assert!(msg.contains("must return a `number` on every path"), "{msg}");
    }

    #[test]
    fn entry_point_rules() {
        let e = check("function Foo(): JSX.Element { return <p></p>; }").unwrap_err();
        assert!(e.message.contains("no root component"));
        let m = check("function App(): JSX.Element { return <p></p>; }").unwrap();
        assert_eq!(m.entry, "App");
        let e = check("export default function Main(n: number): JSX.Element { return <p></p>; }").unwrap_err();
        assert!(e.message.contains("must take no parameters"));
    }

    #[test]
    fn declarations_can_be_used_before_definition() {
        check(
            r#"
            export default function App(): JSX.Element { return <p>{greeting()}{later.value}</p>; }
            function greeting(): string { return "hi"; }
            const later = signal("x");
            "#,
        )
        .unwrap();
    }

    #[test]
    fn imports() {
        let m = check(
            "import { signal } from \"pulse\";\nimport \"./a.css\";\nimport \"pico/css/pico.css\";\nfunction App(): JSX.Element { return <p></p>; }",
        )
        .unwrap();
        assert_eq!(m.stylesheets, ["./a.css", "pico/css/pico.css"]);
        let e = check("import styles from \"./a.css\";\nfunction App(): JSX.Element { return <p></p>; }").unwrap_err();
        assert!(e.message.contains("CSS modules are not supported"), "{}", e.message);
        let e = check("import { x } from \"./other\";\nfunction App(): JSX.Element { return <p></p>; }").unwrap_err();
        assert!(e.message.contains("cannot import `./other`"), "{}", e.message);
    }

    #[test]
    fn recursion_requires_annotation() {
        check("function fact(n: number): number { if (n <= 1) { return 1; } return n * fact(n - 1); }\nfunction App(): JSX.Element { return <p>{fact(5)}</p>; }")
            .unwrap();
        let e = check("function loop(n: number) { return loop(n); }\nfunction App(): JSX.Element { return <p></p>; }")
            .unwrap_err();
        assert!(e.message.contains("add a return type annotation"), "{}", e.message);
    }
}
