//! Recursive-descent parser for the supported TSX subset.
//!
//! Supported today: `import` (ignored), top-level `const`/`let` and
//! `function` declarations (optionally `export`/`export default`), type
//! annotations, arrow functions, `if`/`return`, the usual operators, generic
//! calls such as `signal<number>(0)`, and JSX elements/fragments with string
//! or `{expression}` attributes.

use crate::ast::*;
use crate::diagnostic::{Diagnostic, Result};
use crate::lexer::{decode_entities, Lexer, Tok};

/// Parses a complete `.tsx` module.
pub fn parse(src: &str) -> Result<Program> {
    let mut p = Parser { lx: Lexer::new(src), peeked: None, last_end: 0 };
    let mut items = Vec::new();
    while !p.peek_is_eof()? {
        items.push(p.item()?);
    }
    Ok(Program { items })
}

struct Parser<'a> {
    lx: Lexer<'a>,
    /// One token of lookahead. Raw JSX scanning rewinds it (see `unpeek`).
    peeked: Option<(Tok, Span)>,
    /// End of the last consumed token, for building spans.
    last_end: usize,
}

fn err<T>(msg: impl Into<String>, span: Span) -> Result<T> {
    Err(Diagnostic::new(msg, span))
}

impl<'a> Parser<'a> {
    // ----- token plumbing -------------------------------------------------

    fn peek(&mut self) -> Result<&(Tok, Span)> {
        if self.peeked.is_none() {
            self.peeked = Some(self.lx.next_token()?);
        }
        Ok(self.peeked.as_ref().unwrap())
    }

    fn peek_tok(&mut self) -> Result<Tok> {
        Ok(self.peek()?.0.clone())
    }

    fn peek_span(&mut self) -> Result<Span> {
        Ok(self.peek()?.1)
    }

    fn peek_is_eof(&mut self) -> Result<bool> {
        Ok(self.peek()?.0 == Tok::Eof)
    }

    fn bump(&mut self) -> Result<(Tok, Span)> {
        self.peek()?;
        let t = self.peeked.take().unwrap();
        self.last_end = t.1.end;
        Ok(t)
    }

    fn is_punct(&mut self, p: &str) -> Result<bool> {
        Ok(matches!(&self.peek()?.0, Tok::Punct(q) if *q == p))
    }

    fn is_word(&mut self, w: &str) -> Result<bool> {
        Ok(matches!(&self.peek()?.0, Tok::Ident(q) if q == w))
    }

    fn eat_punct(&mut self, p: &str) -> Result<bool> {
        if self.is_punct(p)? {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect_punct(&mut self, p: &str) -> Result<Span> {
        if self.is_punct(p)? {
            Ok(self.bump()?.1)
        } else {
            let (tok, span) = self.peek()?.clone();
            err(format!("expected `{p}`, found {}", describe(&tok)), span)
        }
    }

    fn ident(&mut self) -> Result<(String, Span)> {
        match self.bump()? {
            (Tok::Ident(name), span) => Ok((name, span)),
            (tok, span) => err(format!("expected a name, found {}", describe(&tok)), span),
        }
    }

    /// Drops the lookahead token and rewinds the lexer to its start, so the
    /// raw JSX scanner can take over at that position.
    fn unpeek(&mut self) {
        if let Some((_, span)) = self.peeked.take() {
            self.lx.pos = span.start;
        }
    }

    fn save(&self) -> (usize, Option<(Tok, Span)>, usize) {
        (self.lx.pos, self.peeked.clone(), self.last_end)
    }

    fn restore(&mut self, s: (usize, Option<(Tok, Span)>, usize)) {
        self.lx.pos = s.0;
        self.peeked = s.1;
        self.last_end = s.2;
    }

    /// Runs `f`; on failure, rewinds as if nothing had been consumed.
    fn attempt<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Option<T> {
        let saved = self.save();
        match f(self) {
            Ok(v) => Some(v),
            Err(_) => {
                self.restore(saved);
                None
            }
        }
    }

    // ----- items and statements --------------------------------------------

    fn item(&mut self) -> Result<Item> {
        let (tok, span) = self.peek()?.clone();
        let Tok::Ident(word) = tok else {
            return err(format!("expected a declaration, found {}", describe(&tok)), span);
        };
        match word.as_str() {
            "import" => {
                // `import { signal } from "pulse";` — consume through the
                // module string. Pulse built-ins are always in scope.
                self.bump()?;
                loop {
                    match self.bump()? {
                        (Tok::Str(_), _) => break,
                        (Tok::Eof, s) => return err("unterminated import", s),
                        _ => {}
                    }
                }
                self.eat_punct(";")?;
                Ok(Item::Import)
            }
            "export" => {
                self.bump()?;
                let is_default = if self.is_word("default")? {
                    self.bump()?;
                    true
                } else {
                    false
                };
                if self.is_word("function")? {
                    Ok(Item::Function(self.function(is_default)?))
                } else if !is_default && (self.is_word("const")? || self.is_word("let")?) {
                    Ok(Item::Var(self.var_decl()?))
                } else {
                    let span = self.peek_span()?;
                    err("only `export function`, `export default function` and `export const` are supported", span)
                }
            }
            "function" => Ok(Item::Function(self.function(false)?)),
            "const" | "let" => Ok(Item::Var(self.var_decl()?)),
            "var" => err("`var` is not supported; use `const` or `let`", span),
            "interface" | "type" | "enum" | "class" => {
                err(format!("`{word}` declarations are not supported yet"), span)
            }
            _ => err(
                "expected `import`, `const`, `let` or `function` at the top level of a module",
                span,
            ),
        }
    }

    fn function(&mut self, is_default_export: bool) -> Result<FnDecl> {
        self.bump()?; // `function`
        let (name, name_span) = self.ident()?;
        self.expect_punct("(")?;
        let params = self.params()?;
        let ret = if self.eat_punct(":")? { Some(self.type_ann()?) } else { None };
        let body = self.block()?;
        Ok(FnDecl { name, name_span, params, ret, body, is_default_export })
    }

    /// Parses `a: T, b: U)` (the opening paren is already consumed).
    fn params(&mut self) -> Result<Vec<Param>> {
        let mut params = Vec::new();
        while !self.eat_punct(")")? {
            let (name, span) = self.ident()?;
            let ty = if self.eat_punct(":")? { Some(self.type_ann()?) } else { None };
            params.push(Param { name, span, ty });
            if !self.is_punct(")")? {
                self.expect_punct(",")?;
            }
        }
        Ok(params)
    }

    fn var_decl(&mut self) -> Result<VarDecl> {
        let (kw, _) = self.bump()?;
        let is_let = kw == Tok::Ident("let".into());
        let (name, name_span) = self.ident()?;
        let ty = if self.eat_punct(":")? { Some(self.type_ann()?) } else { None };
        if !self.is_punct("=")? {
            let span = self.peek_span()?;
            return err("variables must be initialized (`= value`)", span);
        }
        self.bump()?;
        let init = self.expr()?;
        self.eat_punct(";")?;
        Ok(VarDecl { name, name_span, is_let, ty, init })
    }

    fn block(&mut self) -> Result<Vec<Stmt>> {
        self.expect_punct("{")?;
        let mut stmts = Vec::new();
        while !self.eat_punct("}")? {
            if self.peek_is_eof()? {
                let span = self.peek_span()?;
                return err("expected `}`", span);
            }
            stmts.push(self.stmt()?);
        }
        Ok(stmts)
    }

    fn stmt(&mut self) -> Result<Stmt> {
        if self.is_word("const")? || self.is_word("let")? {
            return Ok(Stmt::Var(self.var_decl()?));
        }
        if self.is_word("return")? {
            let (_, span) = self.bump()?;
            let value = if self.is_punct(";")? || self.is_punct("}")? { None } else { Some(self.expr()?) };
            self.eat_punct(";")?;
            return Ok(Stmt::Return(value, span));
        }
        if self.is_word("if")? {
            self.bump()?;
            self.expect_punct("(")?;
            let cond = self.expr()?;
            self.expect_punct(")")?;
            let then = self.block_or_stmt()?;
            let els = if self.is_word("else")? {
                self.bump()?;
                Some(self.block_or_stmt()?)
            } else {
                None
            };
            return Ok(Stmt::If { cond, then, els });
        }
        for kw in ["for", "while", "do", "switch", "try", "throw", "function", "var", "class"] {
            if self.is_word(kw)? {
                let span = self.peek_span()?;
                return err(format!("`{kw}` statements are not supported yet"), span);
            }
        }
        let e = self.expr()?;
        self.eat_punct(";")?;
        Ok(Stmt::Expr(e))
    }

    fn block_or_stmt(&mut self) -> Result<Vec<Stmt>> {
        if self.is_punct("{")? {
            self.block()
        } else {
            Ok(vec![self.stmt()?])
        }
    }

    // ----- types ------------------------------------------------------------

    fn type_ann(&mut self) -> Result<TypeAnn> {
        let first = self.primary_type()?;
        let mut ty = first;
        loop {
            if self.is_punct("[")? {
                self.bump()?;
                let end = self.expect_punct("]")?;
                ty = TypeAnn::Unsupported { span: ty.span().to(end) };
            } else if self.is_punct("|")? || self.is_punct("&")? {
                self.bump()?;
                let rhs = self.primary_type()?;
                ty = TypeAnn::Unsupported { span: ty.span().to(rhs.span()) };
            } else {
                return Ok(ty);
            }
        }
    }

    fn primary_type(&mut self) -> Result<TypeAnn> {
        let (tok, span) = self.peek()?.clone();
        match tok {
            Tok::Punct("(") => {
                self.bump()?;
                let mut params = Vec::new();
                while !self.eat_punct(")")? {
                    self.ident()?;
                    self.expect_punct(":")?;
                    params.push(self.type_ann()?);
                    if !self.is_punct(")")? {
                        self.expect_punct(",")?;
                    }
                }
                self.expect_punct("=>")?;
                let ret = self.type_ann()?;
                let span = span.to(ret.span());
                Ok(TypeAnn::Fn { params, ret: Box::new(ret), span })
            }
            Tok::Ident(mut name) => {
                self.bump()?;
                while self.eat_punct(".")? {
                    let (part, _) = self.ident()?;
                    name = format!("{name}.{part}");
                }
                let mut args = Vec::new();
                if self.eat_punct("<")? {
                    loop {
                        args.push(self.type_ann()?);
                        if !self.eat_punct(",")? {
                            break;
                        }
                    }
                    self.expect_punct(">")?;
                }
                Ok(TypeAnn::Named { name, args, span: span.to(Span::new(0, self.last_end)) })
            }
            Tok::Str(_) | Tok::Num(_) => {
                self.bump()?;
                Ok(TypeAnn::Unsupported { span })
            }
            Tok::Punct("{") => {
                // Object type: skip it, balanced.
                let mut depth = 0;
                loop {
                    match self.bump()? {
                        (Tok::Punct("{"), _) => depth += 1,
                        (Tok::Punct("}"), _) => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        (Tok::Eof, s) => return err("unterminated object type", s),
                        _ => {}
                    }
                }
                Ok(TypeAnn::Unsupported { span: span.to(Span::new(0, self.last_end)) })
            }
            other => err(format!("expected a type, found {}", describe(&other)), span),
        }
    }

    // ----- expressions --------------------------------------------------------

    pub fn expr(&mut self) -> Result<Expr> {
        if let Some(arrow) = self.attempt(Self::arrow_head) {
            return self.arrow_body(arrow);
        }
        let target = self.conditional()?;
        let op = match self.peek_tok()? {
            Tok::Punct("=") => AssignOp::Assign,
            Tok::Punct("+=") => AssignOp::Add,
            Tok::Punct("-=") => AssignOp::Sub,
            Tok::Punct("*=") => AssignOp::Mul,
            Tok::Punct("/=") => AssignOp::Div,
            _ => return Ok(target),
        };
        self.bump()?;
        let value = self.expr()?;
        let span = target.span.to(value.span);
        Ok(Expr { kind: ExprKind::Assign { op, target: Box::new(target), value: Box::new(value) }, span })
    }

    /// Parses the `(params): Ret =>` part of an arrow function.
    fn arrow_head(&mut self) -> Result<(Vec<Param>, Option<TypeAnn>, Span)> {
        let start = self.peek_span()?;
        let params = if self.eat_punct("(")? {
            self.params()?
        } else {
            let (name, span) = self.ident()?;
            vec![Param { name, span, ty: None }]
        };
        let ret = if self.eat_punct(":")? { Some(self.type_ann()?) } else { None };
        self.expect_punct("=>")?;
        Ok((params, ret, start))
    }

    fn arrow_body(&mut self, (params, ret, start): (Vec<Param>, Option<TypeAnn>, Span)) -> Result<Expr> {
        let body = if self.is_punct("{")? {
            ArrowBody::Block(self.block()?)
        } else {
            ArrowBody::Expr(Box::new(self.expr()?))
        };
        let span = start.to(Span::new(0, self.last_end));
        Ok(Expr { kind: ExprKind::Arrow { params, ret, body }, span })
    }

    fn conditional(&mut self) -> Result<Expr> {
        let test = self.binary(0)?;
        if !self.eat_punct("?")? {
            return Ok(test);
        }
        let cons = self.expr()?;
        self.expect_punct(":")?;
        let alt = self.expr()?;
        let span = test.span.to(alt.span);
        Ok(Expr { kind: ExprKind::Cond { test: Box::new(test), cons: Box::new(cons), alt: Box::new(alt) }, span })
    }

    /// Precedence climbing over binary operators.
    fn binary(&mut self, min_prec: u8) -> Result<Expr> {
        let mut left = self.unary()?;
        loop {
            let (tok, span) = self.peek()?.clone();
            let Tok::Punct(p) = tok else { break };
            if p == "??" {
                return err("`??` is not supported yet", span);
            }
            let Some((op, prec)) = binop(p) else { break };
            if prec < min_prec {
                break;
            }
            self.bump()?;
            let right = self.binary(prec + 1)?;
            let span = left.span.to(right.span);
            left = Expr { kind: ExprKind::Binary { op, left: Box::new(left), right: Box::new(right) }, span };
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr> {
        let (tok, span) = self.peek()?.clone();
        let op = match tok {
            Tok::Punct("!") => Some(UnaryOp::Not),
            Tok::Punct("-") => Some(UnaryOp::Neg),
            Tok::Punct("+") => Some(UnaryOp::Plus),
            Tok::Punct(p @ ("++" | "--")) => {
                self.bump()?;
                let target = self.unary()?;
                let span = span.to(target.span);
                let delta = if p == "++" { 1.0 } else { -1.0 };
                return Ok(Expr { kind: ExprKind::Update { delta, target: Box::new(target) }, span });
            }
            _ => None,
        };
        if let Some(op) = op {
            self.bump()?;
            let expr = self.unary()?;
            let span = span.to(expr.span);
            return Ok(Expr { kind: ExprKind::Unary { op, expr: Box::new(expr) }, span });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut e = self.call_member()?;
        loop {
            let (tok, span) = self.peek()?.clone();
            let delta = match tok {
                Tok::Punct("++") => 1.0,
                Tok::Punct("--") => -1.0,
                _ => return Ok(e),
            };
            self.bump()?;
            let span = e.span.to(span);
            e = Expr { kind: ExprKind::Update { delta, target: Box::new(e) }, span };
        }
    }

    fn call_member(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.eat_punct(".")? {
                let (prop, pspan) = self.ident()?;
                let span = e.span.to(pspan);
                e = Expr { kind: ExprKind::Member { obj: Box::new(e), prop }, span };
            } else if self.is_punct("(")? {
                e = self.call(e, Vec::new())?;
            } else if self.is_punct("<")? {
                // Generic call `f<T>(...)`, disambiguated from `a < b` by
                // requiring the type arguments to be followed by `(`.
                let type_args = self.attempt(|p| {
                    p.bump()?;
                    let mut args = vec![p.type_ann()?];
                    while p.eat_punct(",")? {
                        args.push(p.type_ann()?);
                    }
                    p.expect_punct(">")?;
                    if p.is_punct("(")? {
                        Ok(args)
                    } else {
                        err("not a generic call", Span::default())
                    }
                });
                match type_args {
                    Some(args) => e = self.call(e, args)?,
                    None => return Ok(e),
                }
            } else if self.is_punct("?.")? || self.is_punct("[")? {
                let span = self.peek_span()?;
                return err("optional chaining and indexing are not supported yet", span);
            } else {
                return Ok(e);
            }
        }
    }

    fn call(&mut self, callee: Expr, type_args: Vec<TypeAnn>) -> Result<Expr> {
        self.expect_punct("(")?;
        let mut args = Vec::new();
        while !self.eat_punct(")")? {
            args.push(self.expr()?);
            if !self.is_punct(")")? {
                self.expect_punct(",")?;
            }
        }
        let span = callee.span.to(Span::new(0, self.last_end));
        Ok(Expr { kind: ExprKind::Call { callee: Box::new(callee), type_args, args }, span })
    }

    fn primary(&mut self) -> Result<Expr> {
        let (tok, span) = self.peek()?.clone();
        let kind = match tok {
            Tok::Punct("<") => {
                self.unpeek();
                let el = self.jsx_element()?;
                return Ok(Expr { span: el.span, kind: ExprKind::Jsx(el) });
            }
            Tok::Punct("(") => {
                self.bump()?;
                let inner = self.expr()?;
                self.expect_punct(")")?;
                return Ok(inner);
            }
            Tok::Num(n) => ExprKind::Num(n),
            Tok::Str(s) => ExprKind::Str(s),
            Tok::Ident(w) => match w.as_str() {
                "true" => ExprKind::Bool(true),
                "false" => ExprKind::Bool(false),
                "function" | "new" | "this" | "class" | "await" | "async" | "null" | "undefined" => {
                    return err(format!("`{w}` is not supported yet"), span)
                }
                _ => ExprKind::Ident(w),
            },
            other => return err(format!("expected an expression, found {}", describe(&other)), span),
        };
        self.bump()?;
        Ok(Expr { kind, span })
    }

    // ----- JSX ------------------------------------------------------------------

    /// Parses an element starting at `<`. The lookahead must be empty.
    fn jsx_element(&mut self) -> Result<JsxElement> {
        debug_assert!(self.peeked.is_none());
        self.lx.skip_trivia()?;
        let start = self.lx.pos;
        self.lx.eat("<")?;
        let (name, name_span) = if self.lx.eat(">")? {
            (String::new(), Span::new(start, self.lx.pos))
        } else {
            self.lx.jsx_name()?
        };
        let mut attrs = Vec::new();
        let mut self_closing = false;
        if !name.is_empty() {
            loop {
                self.lx.skip_trivia()?;
                if self.lx.eat("/>")? {
                    self_closing = true;
                    break;
                }
                if self.lx.eat(">")? {
                    break;
                }
                if self.lx.at("{") {
                    return err("spread attributes are not supported yet", Span::new(self.lx.pos, self.lx.pos + 1));
                }
                if self.lx.at_end() {
                    return err(format!("unclosed tag `<{name}`"), name_span);
                }
                let (attr, aspan) = self.lx.jsx_name()?;
                let value = if self.lx.eat("=")? {
                    self.lx.skip_trivia()?;
                    if self.lx.eat("{")? {
                        let e = self.expr()?;
                        self.expect_punct("}")?;
                        self.unpeek();
                        JsxAttrValue::Expr(e)
                    } else {
                        JsxAttrValue::Str(self.lx.jsx_attr_string()?)
                    }
                } else {
                    JsxAttrValue::True
                };
                attrs.push(JsxAttr { name: attr, value, span: aspan.to(Span::new(0, self.lx.pos)) });
            }
        }
        let mut children = Vec::new();
        if !self_closing {
            loop {
                let (raw, tspan) = self.lx.jsx_text();
                if let Some(text) = clean_jsx_text(&raw) {
                    children.push(JsxChild::Text(text, tspan));
                }
                if self.lx.at("</") {
                    self.lx.pos += 2;
                    if !name.is_empty() {
                        let (close, cspan) = self.lx.jsx_name()?;
                        if close != name {
                            return err(format!("expected `</{name}>`, found `</{close}>`"), cspan);
                        }
                    }
                    if !self.lx.eat(">")? {
                        return err("expected `>`", Span::new(self.lx.pos, self.lx.pos + 1));
                    }
                    break;
                } else if self.lx.at("<") {
                    children.push(JsxChild::Element(self.jsx_element()?));
                } else if self.lx.at("{") {
                    self.lx.pos += 1;
                    // `{}` and `{/* comment */}` produce nothing.
                    if self.lx.eat("}")? {
                        continue;
                    }
                    let e = self.expr()?;
                    self.expect_punct("}")?;
                    self.unpeek();
                    children.push(JsxChild::Expr(e));
                } else {
                    let what = if name.is_empty() { "fragment".to_string() } else { format!("`<{name}>`") };
                    return err(format!("unclosed {what}"), name_span);
                }
            }
        }
        let span = Span::new(start, self.lx.pos);
        self.last_end = self.lx.pos;
        Ok(JsxElement { name, attrs, children, span })
    }
}

/// Applies React's JSX whitespace rules: lines are trimmed, whitespace-only
/// lines are dropped and the remaining lines are joined with a single space.
fn clean_jsx_text(raw: &str) -> Option<String> {
    if !raw.contains('\n') {
        return (!raw.is_empty()).then(|| decode_entities(raw));
    }
    let lines: Vec<&str> = raw.split('\n').collect();
    let last = lines.len() - 1;
    let parts: Vec<&str> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line = if i > 0 { line.trim_start() } else { line };
            if i < last { line.trim_end() } else { line }
        })
        .filter(|l| !l.is_empty())
        .collect();
    (!parts.is_empty()).then(|| decode_entities(&parts.join(" ")))
}

fn binop(p: &str) -> Option<(BinOp, u8)> {
    Some(match p {
        "||" => (BinOp::Or, 1),
        "&&" => (BinOp::And, 2),
        "==" | "===" => (BinOp::Eq, 3),
        "!=" | "!==" => (BinOp::Ne, 3),
        "<" => (BinOp::Lt, 4),
        "<=" => (BinOp::Le, 4),
        ">" => (BinOp::Gt, 4),
        ">=" => (BinOp::Ge, 4),
        "+" => (BinOp::Add, 5),
        "-" => (BinOp::Sub, 5),
        "*" => (BinOp::Mul, 6),
        "/" => (BinOp::Div, 6),
        "%" => (BinOp::Rem, 6),
        _ => return None,
    })
}

fn describe(tok: &Tok) -> String {
    match tok {
        Tok::Ident(w) => format!("`{w}`"),
        Tok::Num(n) => format!("number `{n}`"),
        Tok::Str(_) => "a string".into(),
        Tok::Punct(p) => format!("`{p}`"),
        Tok::Eof => "end of file".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_expr(src: &str) -> Expr {
        let program = parse(&format!("const x = {src};")).unwrap();
        match program.items.into_iter().next().unwrap() {
            Item::Var(v) => v.init,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn precedence() {
        let e = parse_expr("1 + 2 * 3 < 10 && !done");
        let ExprKind::Binary { op: BinOp::And, left, .. } = e.kind else { panic!() };
        let ExprKind::Binary { op: BinOp::Lt, left, .. } = left.kind else { panic!() };
        let ExprKind::Binary { op: BinOp::Add, right, .. } = left.kind else { panic!() };
        assert!(matches!(right.kind, ExprKind::Binary { op: BinOp::Mul, .. }));
    }

    #[test]
    fn generic_call_vs_less_than() {
        let e = parse_expr("signal<number>(0)");
        let ExprKind::Call { type_args, .. } = e.kind else { panic!("{e:?}") };
        assert_eq!(type_args.len(), 1);
        let e = parse_expr("a < b");
        assert!(matches!(e.kind, ExprKind::Binary { op: BinOp::Lt, .. }));
    }

    #[test]
    fn arrows_and_updates() {
        let e = parse_expr("() => count.value++");
        let ExprKind::Arrow { params, body: ArrowBody::Expr(body), .. } = e.kind else { panic!() };
        assert!(params.is_empty());
        assert!(matches!(body.kind, ExprKind::Update { delta, .. } if delta == 1.0));
        let e = parse_expr("(): void => { a.value += 2; }");
        assert!(matches!(e.kind, ExprKind::Arrow { body: ArrowBody::Block(_), ret: Some(_), .. }));
        // A parenthesized expression is not mistaken for an arrow.
        assert!(matches!(parse_expr("(a + b) * 2").kind, ExprKind::Binary { op: BinOp::Mul, .. }));
    }

    #[test]
    fn jsx_tree() {
        let e = parse_expr(
            r#"<div class="app" id={name}>
                 <h1>Hello,  world &amp; you</h1>
                 Count: {count.value}
                 <input disabled />
                 {/* comment */}
               </div>"#,
        );
        let ExprKind::Jsx(div) = e.kind else { panic!() };
        assert_eq!(div.name, "div");
        assert_eq!(div.attrs.len(), 2);
        assert!(matches!(&div.attrs[0].value, JsxAttrValue::Str(s) if s == "app"));
        assert!(matches!(&div.attrs[1].value, JsxAttrValue::Expr(_)));
        assert_eq!(div.children.len(), 4, "{:#?}", div.children);
        let JsxChild::Element(h1) = &div.children[0] else { panic!() };
        assert!(matches!(&h1.children[0], JsxChild::Text(t, _) if t == "Hello,  world & you"));
        assert!(matches!(&div.children[1], JsxChild::Text(t, _) if t == "Count: "));
        assert!(matches!(&div.children[2], JsxChild::Expr(_)));
        let JsxChild::Element(input) = &div.children[3] else { panic!() };
        assert!(matches!(input.attrs[0].value, JsxAttrValue::True));
    }

    #[test]
    fn fragments_and_mismatched_tags() {
        let ExprKind::Jsx(f) = parse_expr("<><b>x</b></>").kind else { panic!() };
        assert_eq!(f.name, "");
        assert_eq!(f.children.len(), 1);
        let e = parse("const x = <a></b>;").unwrap_err();
        assert!(e.message.contains("expected `</a>`"), "{}", e.message);
    }

    #[test]
    fn full_component() {
        let p = parse(
            r#"
            import { signal } from "pulse";
            const count = signal<number>(0);
            export default function App(): JSX.Element {
              const label: string = "Count";
              return <button onClick={() => count.value++}>{label}: {count.value}</button>;
            }
            "#,
        )
        .unwrap();
        assert_eq!(p.items.len(), 3);
        let Item::Function(f) = &p.items[2] else { panic!() };
        assert!(f.is_default_export);
        assert!(matches!(&f.ret, Some(TypeAnn::Named { name, .. }) if name == "JSX.Element"));
        assert_eq!(f.body.len(), 2);
    }

    #[test]
    fn jsx_text_whitespace() {
        assert_eq!(clean_jsx_text("\n   \n  "), None);
        assert_eq!(clean_jsx_text("  a  "), Some("  a  ".into()));
        assert_eq!(clean_jsx_text("\n  Hello\n  world  \n"), Some("Hello world".into()));
        assert_eq!(clean_jsx_text("Count: "), Some("Count: ".into()));
    }
}
