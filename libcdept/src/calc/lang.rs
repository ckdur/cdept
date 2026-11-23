//! The expression language: lexer, parser, syntax tree.
//!
//! ```text
//! expr   = term { ("+" | "-") term }
//! term   = unary { ("*" | "/") unary }
//! unary  = ("-" | "+") unary | power
//! power  = primary [ "^" unary ]
//! primary = number | string | name | name "(" [ arg { "," arg } ] ")" | "(" expr ")"
//! arg    = [ name "=" ] expr
//! ```
//!
//! Numbers take SPICE suffixes (`1u`, `5p`, `1meg`, `10pF`), strings are `"…"` or
//! `'…'`, names are case-insensitive.

use std::fmt;

/// A byte range of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// A parse or evaluation error with the place it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct CalcError {
    pub msg: String,
    pub span: Option<Span>,
}

impl CalcError {
    pub fn new(msg: impl Into<String>, span: Span) -> CalcError {
        CalcError {
            msg: msg.into(),
            span: Some(span),
        }
    }

    /// The message with the line of `src` and a caret under the error.
    pub fn render(&self, src: &str) -> String {
        let Some(span) = self.span else {
            return self.msg.clone();
        };
        let start = span.start.min(src.len());
        let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = src[start..].find('\n').map_or(src.len(), |i| start + i);
        let col = src[line_start..start].chars().count();
        let width = src[start..span.end.clamp(start, line_end)]
            .chars()
            .count()
            .max(1);
        format!(
            "{}\n  {}\n  {}{}",
            self.msg,
            &src[line_start..line_end],
            " ".repeat(col),
            "^".repeat(width)
        )
    }

    /// 1-based line and column of the start.
    pub fn position(&self, src: &str) -> Option<(usize, usize)> {
        let span = self.span?;
        let start = span.start.min(src.len());
        let line = src[..start].matches('\n').count() + 1;
        let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
        Some((line, src[line_start..start].chars().count() + 1))
    }
}

impl fmt::Display for CalcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.span {
            Some(s) => write!(f, "{} (at byte {})", self.msg, s.start),
            None => write!(f, "{}", self.msg),
        }
    }
}

impl std::error::Error for CalcError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Num(f64),
    Str(String),
    /// A variable of the run point (lower case).
    Var(String),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call {
        name: String,
        args: Vec<Arg>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
    pub span: Span,
}

/// A number with an optional SPICE suffix (`1u`, `2.5meg`, `10pF`, `1e-9`); None when
/// `s` is not one.
pub fn parse_spice_number(s: &str) -> Option<f64> {
    let s = s.trim();
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == digits || (i == digits + 1 && b[digits] == b'.') {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let value: f64 = s[..i].parse().ok()?;
    let rest = &s[i..];
    if !rest.chars().all(|c| c.is_ascii_alphabetic() || c == 'µ') {
        return None;
    }
    let lower = rest.to_lowercase();
    let scale = if lower.starts_with("meg") {
        1e6
    } else if lower.starts_with("mil") {
        25.4e-6
    } else {
        match lower.chars().next() {
            Some('t') => 1e12,
            Some('g') => 1e9,
            Some('k') => 1e3,
            Some('m') => 1e-3,
            Some('u') | Some('µ') => 1e-6,
            Some('n') => 1e-9,
            Some('p') => 1e-12,
            Some('f') => 1e-15,
            _ => 1.0,
        }
    };
    Some(value * scale)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
    Comma,
    Eq,
    End,
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Num(_) => "a number".into(),
        Tok::Str(_) => "a string".into(),
        Tok::Ident(n) => format!("'{n}'"),
        Tok::Plus => "'+'".into(),
        Tok::Minus => "'-'".into(),
        Tok::Star => "'*'".into(),
        Tok::Slash => "'/'".into(),
        Tok::Caret => "'^'".into(),
        Tok::LParen => "'('".into(),
        Tok::RParen => "')'".into(),
        Tok::Comma => "','".into(),
        Tok::Eq => "'='".into(),
        Tok::End => "the end".into(),
    }
}

/// Where the number that starts at `i` ends: digits, a fraction, an exponent, then the
/// letters of a suffix and unit.
fn scan_number(src: &str, i: usize) -> usize {
    let b = src.as_bytes();
    let mut j = i;
    while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'.') {
        j += 1;
    }
    if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
        let mut k = j + 1;
        if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
            k += 1;
        }
        if k < b.len() && b[k].is_ascii_digit() {
            while k < b.len() && b[k].is_ascii_digit() {
                k += 1;
            }
            j = k;
        }
    }
    for (off, ch) in src[j..].char_indices() {
        if !(ch.is_ascii_alphabetic() || ch == 'µ') {
            return j + off;
        }
    }
    src.len()
}

fn lex(src: &str) -> Result<Vec<(Tok, Span)>, CalcError> {
    let mut out = vec![];
    let mut it = src.char_indices().peekable();
    while let Some(&(i, c)) = it.peek() {
        let single = |t: Tok, len: usize| {
            (
                t,
                Span {
                    start: i,
                    end: i + len,
                },
            )
        };
        match c {
            c if c.is_whitespace() => {
                it.next();
            }
            '+' | '-' | '*' | '/' | '^' | '(' | ')' | ',' | '=' => {
                it.next();
                out.push(single(
                    match c {
                        '+' => Tok::Plus,
                        '-' => Tok::Minus,
                        '*' => Tok::Star,
                        '/' => Tok::Slash,
                        '^' => Tok::Caret,
                        '(' => Tok::LParen,
                        ')' => Tok::RParen,
                        ',' => Tok::Comma,
                        _ => Tok::Eq,
                    },
                    1,
                ));
            }
            '"' | '\'' => {
                it.next();
                let mut s = String::new();
                let mut closed = None;
                for (j, d) in it.by_ref() {
                    if d == c {
                        closed = Some(j);
                        break;
                    }
                    s.push(d);
                }
                let Some(end) = closed else {
                    return Err(CalcError::new(
                        "unterminated string",
                        Span {
                            start: i,
                            end: src.len(),
                        },
                    ));
                };
                out.push((
                    Tok::Str(s),
                    Span {
                        start: i,
                        end: end + 1,
                    },
                ));
            }
            c if c.is_ascii_digit() || c == '.' => {
                let end = scan_number(src, i);
                while it.peek().is_some_and(|&(j, _)| j < end) {
                    it.next();
                }
                let text = &src[i..end];
                match parse_spice_number(text) {
                    Some(v) => out.push((Tok::Num(v), Span { start: i, end })),
                    None => {
                        return Err(CalcError::new(
                            format!("'{text}' is not a number"),
                            Span { start: i, end },
                        ));
                    }
                }
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut end = i;
                while let Some(&(j, d)) = it.peek() {
                    if d.is_alphanumeric() || d == '_' || d == '.' || d == '#' {
                        end = j + d.len_utf8();
                        it.next();
                    } else {
                        break;
                    }
                }
                out.push((
                    Tok::Ident(src[i..end].to_lowercase()),
                    Span { start: i, end },
                ));
            }
            _ => {
                return Err(CalcError::new(
                    format!("unexpected character '{c}'"),
                    Span {
                        start: i,
                        end: i + c.len_utf8(),
                    },
                ));
            }
        }
    }
    out.push((
        Tok::End,
        Span {
            start: src.len(),
            end: src.len(),
        },
    ));
    Ok(out)
}

struct Parser {
    toks: Vec<(Tok, Span)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }
    fn span(&self) -> Span {
        self.toks[self.pos].1
    }
    fn bump(&mut self) -> (Tok, Span) {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }
    fn expect(&mut self, t: Tok) -> Result<Span, CalcError> {
        if *self.peek() == t {
            Ok(self.bump().1)
        } else {
            Err(CalcError::new(
                format!("expected {}, found {}", describe(&t), describe(self.peek())),
                self.span(),
            ))
        }
    }

    fn expr(&mut self) -> Result<Expr, CalcError> {
        let mut left = self.term()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.term()?;
            let span = Span {
                start: left.span.start,
                end: right.span.end,
            };
            left = Expr {
                kind: ExprKind::Bin(op, Box::new(left), Box::new(right)),
                span,
            };
        }
    }

    fn term(&mut self) -> Result<Expr, CalcError> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.unary()?;
            let span = Span {
                start: left.span.start,
                end: right.span.end,
            };
            left = Expr {
                kind: ExprKind::Bin(op, Box::new(left), Box::new(right)),
                span,
            };
        }
    }

    fn unary(&mut self) -> Result<Expr, CalcError> {
        match self.peek() {
            Tok::Minus => {
                let start = self.bump().1.start;
                let inner = self.unary()?;
                let span = Span {
                    start,
                    end: inner.span.end,
                };
                Ok(Expr {
                    kind: ExprKind::Neg(Box::new(inner)),
                    span,
                })
            }
            Tok::Plus => {
                self.bump();
                self.unary()
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Expr, CalcError> {
        let base = self.primary()?;
        if *self.peek() == Tok::Caret {
            self.bump();
            let exp = self.unary()?;
            let span = Span {
                start: base.span.start,
                end: exp.span.end,
            };
            return Ok(Expr {
                kind: ExprKind::Bin(BinOp::Pow, Box::new(base), Box::new(exp)),
                span,
            });
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Expr, CalcError> {
        let (tok, span) = self.bump();
        match tok {
            Tok::Num(v) => Ok(Expr {
                kind: ExprKind::Num(v),
                span,
            }),
            Tok::Str(s) => Ok(Expr {
                kind: ExprKind::Str(s),
                span,
            }),
            Tok::LParen => {
                let inner = self.expr()?;
                let close = self.expect(Tok::RParen)?;
                Ok(Expr {
                    kind: inner.kind,
                    span: Span {
                        start: span.start,
                        end: close.end,
                    },
                })
            }
            Tok::Ident(name) => {
                if *self.peek() != Tok::LParen {
                    return Ok(Expr {
                        kind: ExprKind::Var(name),
                        span,
                    });
                }
                self.bump();
                let mut args: Vec<Arg> = vec![];
                if *self.peek() != Tok::RParen {
                    loop {
                        let named = matches!(self.peek(), Tok::Ident(_))
                            && self.toks.get(self.pos + 1).is_some_and(|t| t.0 == Tok::Eq);
                        let start = self.span().start;
                        let arg_name = if named {
                            let Tok::Ident(n) = self.bump().0 else {
                                unreachable!()
                            };
                            self.bump();
                            Some(n)
                        } else {
                            None
                        };
                        let value = self.expr()?;
                        if arg_name.is_none() && args.iter().any(|a| a.name.is_some()) {
                            return Err(CalcError::new(
                                "a positional argument cannot follow a named one",
                                value.span,
                            ));
                        }
                        let aspan = Span {
                            start,
                            end: value.span.end,
                        };
                        args.push(Arg {
                            name: arg_name,
                            value,
                            span: aspan,
                        });
                        if *self.peek() == Tok::Comma {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                let close = self.expect(Tok::RParen)?;
                Ok(Expr {
                    kind: ExprKind::Call { name, args },
                    span: Span {
                        start: span.start,
                        end: close.end,
                    },
                })
            }
            other => Err(CalcError::new(
                format!("expected a value, found {}", describe(&other)),
                span,
            )),
        }
    }
}

/// Parse an expression.
pub fn parse(src: &str) -> Result<Expr, CalcError> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    if *p.peek() == Tok::End {
        return Err(CalcError::new("empty expression", p.span()));
    }
    let e = p.expr()?;
    if *p.peek() != Tok::End {
        return Err(CalcError::new(
            format!("unexpected {} after the expression", describe(p.peek())),
            p.span(),
        ));
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spice_numbers() {
        for (s, v) in [
            ("1u", 1e-6),
            ("5p", 5e-12),
            ("1meg", 1e6),
            ("1M", 1e-3),
            ("10pF", 1e-11),
            ("1e-9", 1e-9),
            ("2.5k", 2500.0),
            (".5", 0.5),
            ("3", 3.0),
            ("1.2e3", 1200.0),
            ("1G", 1e9),
            ("2f", 2e-15),
            ("1mil", 25.4e-6),
        ] {
            let got = parse_spice_number(s).unwrap_or_else(|| panic!("{s}"));
            assert!((got - v).abs() <= v.abs() * 1e-12, "{s}: {got}");
        }
        assert_eq!(parse_spice_number("abc"), None);
        assert_eq!(parse_spice_number("1.2.3"), None);
        assert_eq!(parse_spice_number("."), None);
        assert_eq!(parse_spice_number("1e"), Some(1.0));
    }

    #[test]
    fn parses_calls_named_args_and_precedence() {
        let e = parse("delay(v(\"in\"), v('out'), 0.6, trig_edge=\"rise\") * 2 - 1u").unwrap();
        let ExprKind::Bin(BinOp::Sub, l, r) = &e.kind else {
            panic!("{e:?}")
        };
        assert_eq!(r.kind, ExprKind::Num(1e-6));
        let ExprKind::Bin(BinOp::Mul, call, _) = &l.kind else {
            panic!()
        };
        let ExprKind::Call { name, args } = &call.kind else {
            panic!()
        };
        assert_eq!(name, "delay");
        assert_eq!(args.len(), 4);
        assert_eq!(args[3].name.as_deref(), Some("trig_edge"));
        // -2^2 is -(2^2); 2^3^2 is 2^(3^2).
        let ExprKind::Neg(p) = &parse("-2^2").unwrap().kind else {
            panic!()
        };
        assert!(matches!(p.kind, ExprKind::Bin(BinOp::Pow, _, _)));
        let ExprKind::Bin(BinOp::Pow, _, r) = parse("2^3^2").unwrap().kind else {
            panic!()
        };
        assert!(matches!(r.kind, ExprKind::Bin(BinOp::Pow, _, _)));
        assert_eq!(parse("1e-9+2").unwrap().span, Span { start: 0, end: 6 });
    }

    #[test]
    fn errors_have_positions() {
        let src = "avg(v(\"a\"), )";
        let e = parse(src).unwrap_err();
        assert_eq!(e.position(src), Some((1, 13)), "{e}");
        assert!(e.render(src).contains('^'));
        assert!(parse("1 +").is_err());
        assert!(parse("f(a=1, 2)").is_err());
        assert!(parse("\"open").is_err());
        assert!(parse("2 3").is_err());
        assert!(parse("").is_err());
        assert!(parse("3xy").is_ok(), "units are ignored");
        assert!(parse("1.2.3").is_err());
        assert!(parse("a $ b").is_err());
    }
}
