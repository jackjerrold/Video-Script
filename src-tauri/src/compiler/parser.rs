//! Stage 2: parsing (tokens -> AST).
//!
//! Recursive descent with precedence climbing for expressions.
//!
//! Grammar:
//! ```text
//! program    = { item } ;
//! item       = "video" block
//!            | "let" IDENT "=" expr ";"
//!            | "at" expr IDENT [ "ease" IDENT ] block
//!            | IDENT IDENT block                   (* shape declaration *)
//!            | group ;
//! group      = "group" IDENT "{" { prop | IDENT IDENT block | group } "}" ;
//! block      = "{" { prop } "}" ;
//! prop       = IDENT ":" expr [ "ease" IDENT ] ";" ;
//! expr       = term { ("+" | "-") term } ;
//! term       = unary { ("*" | "/") unary } ;
//! unary      = "-" unary | primary ;
//! primary    = NUMBER | TIME | COLOR | STRING
//!            | IDENT [ "(" [ expr { "," expr } ] ")" ]
//!            | "(" expr ")" ;
//! ```
//!
//! Error recovery: on a syntax error inside a block we skip to the next `;`
//! and keep parsing the block; on an error elsewhere we skip to the end of the
//! current item. That way one typo does not hide every later error.

use super::ast::*;
use super::diagnostics::Diagnostic;
use super::span::Span;
use super::token::{Token, TokenKind};

type PResult<T> = Result<T, ()>;

pub fn parse(tokens: Vec<Token>) -> (Program, Vec<Diagnostic>) {
    let mut p = Parser { toks: tokens, pos: 0, diags: Vec::new() };
    let program = p.parse_program();
    (program, p.diags)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diagnostic>,
}

impl Parser {
    // ---- token helpers -------------------------------------------------

    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if t.kind != TokenKind::Eof {
            self.pos += 1;
        }
        t
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(kind)
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn prev_span(&self) -> Span {
        if self.pos > 0 {
            self.toks[self.pos - 1].span
        } else {
            self.peek().span
        }
    }

    fn error<T>(&mut self, span: Span, msg: impl Into<String>) -> PResult<T> {
        self.diags.push(Diagnostic::error("P001", span, msg));
        Err(())
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> PResult<Token> {
        if self.check(kind) {
            Ok(self.bump())
        } else {
            let found = self.peek().kind.describe();
            let span = self.peek().span;
            self.error(span, format!("expected {what}, found {found}"))
        }
    }

    fn expect_ident(&mut self, what: &str) -> PResult<Ident> {
        if let TokenKind::Ident(name) = self.peek().kind.clone() {
            let span = self.bump().span;
            Ok(Ident { name, span })
        } else {
            let found = self.peek().kind.describe();
            let span = self.peek().span;
            self.error(span, format!("expected {what}, found {found}"))
        }
    }

    /// A missing `;` is reported at the end of the previous token, which is
    /// where the user needs to add it (not at the start of the next line).
    fn expect_semi(&mut self) -> PResult<Span> {
        if self.check(&TokenKind::Semi) {
            Ok(self.bump().span)
        } else {
            let span = self.prev_span();
            self.error(span, "expected ';'")
        }
    }

    // ---- program and items ---------------------------------------------

    fn parse_program(&mut self) -> Program {
        let mut items = Vec::new();
        while !self.check(&TokenKind::Eof) {
            let before = self.pos;
            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(()) => {
                    self.sync_item();
                    if self.pos == before {
                        self.bump(); // guarantee progress
                    }
                }
            }
        }
        Program { items }
    }

    /// Skip to the end of the broken item (past its `;` or its closing `}`),
    /// or to the next item keyword.
    fn sync_item(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.peek().kind {
                TokenKind::Eof => return,
                TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RBrace => {
                    self.bump();
                    if depth <= 1 {
                        return;
                    }
                    depth -= 1;
                }
                TokenKind::Semi => {
                    self.bump();
                    if depth == 0 {
                        return;
                    }
                }
                TokenKind::Let | TokenKind::At | TokenKind::AtSign | TokenKind::Video | TokenKind::Group if depth == 0 => return,
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn parse_item(&mut self) -> PResult<Item> {
        let start = self.peek().span;
        match self.peek().kind.clone() {
            TokenKind::Video => {
                self.bump();
                let body = self.parse_block()?;
                Ok(Item { span: start.to(body.span), kind: ItemKind::Video(body) })
            }
            TokenKind::AtSign => {
                self.bump();
                let name = self.expect_ident("an attribute name such as 'slider'")?;
                let mut args = Vec::new();
                if self.eat(&TokenKind::LParen) {
                    if !self.check(&TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(&TokenKind::RParen, "')'")?;
                }
                let attr = Attr { name, args, span: start.to(self.prev_span()) };
                if !self.check(&TokenKind::Let) {
                    let found = self.peek().kind.describe();
                    let span = self.peek().span;
                    return self.error(span, format!("an attribute must be followed by 'let', found {found}"));
                }
                self.parse_let(Some(attr), start)
            }
            TokenKind::Let => self.parse_let(None, start),
            TokenKind::At => {
                self.bump();
                let time = self.parse_expr()?;
                let target = self.expect_ident("a shape name")?;
                let ease = self.parse_opt_ease()?;
                let body = self.parse_block()?;
                Ok(Item { span: start.to(body.span), kind: ItemKind::At { time, target, ease, body } })
            }
            TokenKind::Group => self.parse_group(),
            TokenKind::Ident(_) => {
                let kind = self.expect_ident("a shape kind")?;
                let name = self.expect_ident("a name for the shape")?;
                let body = self.parse_block()?;
                Ok(Item { span: start.to(body.span), kind: ItemKind::Shape { kind, name, body } })
            }
            other => self.error(
                start,
                format!(
                    "expected a declaration ('video', 'let', 'at', or a shape such as 'rect'), found {}",
                    other.describe()
                ),
            ),
        }
    }

    /// `group name { prop: value; child shapes and groups }`
    ///
    /// Inside the body, what comes next decides what it is: `IDENT :` is a
    /// property, `IDENT IDENT` starts a child shape, `group` a nested group.
    fn parse_let(&mut self, attr: Option<Attr>, start: Span) -> PResult<Item> {
        self.bump(); // 'let'
        let name = self.expect_ident("a variable name")?;
        self.expect(&TokenKind::Eq, "'='")?;
        let value = self.parse_expr()?;
        let end = self.expect_semi()?;
        Ok(Item { span: start.to(end), kind: ItemKind::Let { name, value, attr } })
    }

    fn parse_group(&mut self) -> PResult<Item> {
        let start = self.bump().span; // 'group'
        let name = self.expect_ident("a name for the group")?;
        let open = self.expect(&TokenKind::LBrace, "'{'")?;
        let mut props = Vec::new();
        let mut children = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            let second = self.toks.get(self.pos + 1).map(|t| t.kind.clone());
            match (self.peek().kind.clone(), second) {
                (TokenKind::Ident(_), Some(TokenKind::Colon)) => match self.parse_prop() {
                    Ok(p) => props.push(p),
                    Err(()) => self.sync_prop(),
                },
                (TokenKind::Ident(_), Some(TokenKind::Ident(_))) | (TokenKind::Group, _) => {
                    match self.parse_item() {
                        Ok(item) => children.push(item),
                        Err(()) => self.sync_item(),
                    }
                }
                (other, _) => {
                    let span = self.peek().span;
                    let mut d = Diagnostic::error(
                        "P001",
                        span,
                        format!("expected a property, a shape or a group inside this group, found {}", other.describe()),
                    );
                    if matches!(other, TokenKind::At | TokenKind::Let | TokenKind::Video) {
                        d = d.with_help("'at', 'let' and 'video' go at the top level, outside any group");
                    }
                    self.diags.push(d);
                    self.sync_prop();
                }
            }
        }
        let close = self.expect(&TokenKind::RBrace, "'}'")?;
        let span = start.to(close.span);
        Ok(Item { span, kind: ItemKind::Group { name, body: GroupBody { props, children, span: open.span.to(close.span) } } })
    }

    /// `ease NAME` is a contextual keyword: only meaningful right here.
    fn parse_opt_ease(&mut self) -> PResult<Option<Ident>> {
        match self.peek().kind.clone() {
            TokenKind::Ident(n) if n == "ease" => {
                self.bump();
                Ok(Some(self.expect_ident("an easing name after 'ease'")?))
            }
            _ => Ok(None),
        }
    }

    // ---- blocks and properties -----------------------------------------

    fn parse_block(&mut self) -> PResult<Block> {
        let open = self.expect(&TokenKind::LBrace, "'{'")?;
        let mut props = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            match self.parse_prop() {
                Ok(p) => props.push(p),
                Err(()) => self.sync_prop(),
            }
        }
        let close = self.expect(&TokenKind::RBrace, "'}'")?;
        Ok(Block { props, span: open.span.to(close.span) })
    }

    /// Skip past the next `;`, but never past a `}` (it ends the block).
    fn sync_prop(&mut self) {
        loop {
            match self.peek().kind {
                TokenKind::Semi => {
                    self.bump();
                    return;
                }
                TokenKind::RBrace | TokenKind::Eof => return,
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn parse_prop(&mut self) -> PResult<PropAssign> {
        let name = self.expect_ident("a property name")?;
        self.expect(&TokenKind::Colon, "':' after the property name")?;
        let value = self.parse_expr()?;
        let ease = self.parse_opt_ease()?;
        // The last property may omit its ';' before the closing '}'.
        let end = if self.check(&TokenKind::RBrace) { self.prev_span() } else { self.expect_semi()? };
        Ok(PropAssign { span: name.span.to(end), name, value, ease })
    }

    // ---- expressions ---------------------------------------------------

    fn parse_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_term()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_term()?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr { kind: ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span };
        }
        Ok(lhs)
    }

    fn parse_term(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_unary()?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr { kind: ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        if self.check(&TokenKind::Minus) {
            let minus = self.bump();
            let operand = self.parse_unary()?;
            let span = minus.span.to(operand.span);
            return Ok(Expr { kind: ExprKind::Unary { op: UnOp::Neg, expr: Box::new(operand) }, span });
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Number(v) | TokenKind::Time(v) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Number(v), span: tok.span })
            }
            TokenKind::Color(c) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Color(c), span: tok.span })
            }
            TokenKind::Str(s) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Str(s), span: tok.span })
            }
            TokenKind::Ident(name) => {
                self.bump();
                if self.eat(&TokenKind::LParen) {
                    let mut args = Vec::new();
                    if !self.check(&TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    let close = self.expect(&TokenKind::RParen, "')'")?;
                    let callee = Ident { name, span: tok.span };
                    Ok(Expr { kind: ExprKind::Call { callee, args }, span: tok.span.to(close.span) })
                } else {
                    Ok(Expr { kind: ExprKind::Ident(name), span: tok.span })
                }
            }
            TokenKind::LParen => {
                self.bump();
                let inner = self.parse_expr()?;
                let close = self.expect(&TokenKind::RParen, "')'")?;
                Ok(Expr { kind: inner.kind, span: tok.span.to(close.span) })
            }
            other => self.error(tok.span, format!("expected a value, found {}", other.describe())),
        }
    }
}
