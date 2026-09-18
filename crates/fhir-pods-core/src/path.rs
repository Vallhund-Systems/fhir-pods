//! FHIRPath — the expression language FHIR invariants are written in. The
//! supported core: navigation (choice-type fallback, `extension(url)`),
//! `where` / `select` / `all` / `iif` / `exists` / `empty` / `not` / `count` /
//! `first` / `last` / `single` / `hasValue` / `distinct` / `isDistinct` /
//! `descendants` / `repeat` / `intersect` / `trace` / the `allTrue` family,
//! `and` / `or` / `xor` / `implies`, `=` / `!=`, `<` `<=` `>` `>=` (integers,
//! strings, dates as instants), `+` `-` `*` `div` `mod`, `|`, `in` /
//! `contains`, `is` / `as` / `ofType` against the element's FHIR type, string
//! and conversion functions, `matches` on a linear-time regex, `%resource`,
//! string / integer / boolean literals, `$this`. Anything else fails to parse
//! or evaluates as [`Unsupported`] and the invariant is skipped — never
//! guessed. Three-valued logic per the spec: an empty result is "unknown", and
//! an invariant fails only on a definite `false`.

use alloc::{boxed::Box, string::String, vec::Vec};

use crate::document::Document;
use crate::temporal;

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Str(String),
    Int(i64),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    And,
    Or,
    Xor,
    Implies,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    /// Integer division (`div`). `/` yields decimals and is outside the core.
    Div,
    Mod,
    /// `|` — union of two collections, distinct.
    Union,
    /// `&` — string concatenation, an empty operand reading as `''`.
    Concat,
    /// `x in coll` (and `coll contains x`, reversed at parse time).
    In,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    This,
    /// An environment variable: `%resource`.
    Var(String),
    Lit(Lit),
    /// `base.name` — navigation.
    Nav(Box<Expr>, String),
    /// `base.name(args)` — a function call.
    Call(Box<Expr>, String, Vec<Expr>),
    Bin(Box<Expr>, Op, Box<Expr>),
    /// `e is Type` — a type test (only `$this`'s type is known: the
    /// invariant's element type, from the profile).
    Is(Box<Expr>, String),
    /// `e as Type` — `e` if it is of the type, else empty.
    As(Box<Expr>, String),
}

/// Syntax or semantics outside the supported core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported;

impl core::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("outside the supported core")
    }
}

impl core::error::Error for Unsupported {}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    /// `%name` — an environment variable.
    Var(String),
    Str(String),
    Int(i64),
    LParen,
    RParen,
    Dot,
    Comma,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    /// `|` — union.
    Pipe,
    Amp,
    /// Anything we don't support (`/`, `&`, `~`, …) — makes the parse fail.
    Other,
}

fn tokenize(src: &str) -> Result<Vec<Tok>, Unsupported> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            b')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            b'.' => {
                out.push(Tok::Dot);
                i += 1;
            }
            b',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            b'=' => {
                out.push(Tok::Eq);
                i += 1;
            }
            b'!' if b.get(i + 1) == Some(&b'=') => {
                out.push(Tok::Ne);
                i += 2;
            }
            b'<' | b'>' => {
                let eq = b.get(i + 1) == Some(&b'=');
                out.push(match (c, eq) {
                    (b'<', false) => Tok::Lt,
                    (b'<', true) => Tok::Le,
                    (_, false) => Tok::Gt,
                    (_, true) => Tok::Ge,
                });
                i += if eq { 2 } else { 1 };
            }
            b'+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            b'-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            b'*' => {
                out.push(Tok::Star);
                i += 1;
            }
            b'|' => {
                out.push(Tok::Pipe);
                i += 1;
            }
            b'&' => {
                out.push(Tok::Amp);
                i += 1;
            }
            b'`' => {
                // A delimited identifier: an element named like a keyword (`text.`div``).
                let start = i + 1;
                let mut j = start;
                while j < b.len() && b[j] != b'`' {
                    j += 1;
                }
                if j >= b.len() || j == start {
                    return Err(Unsupported);
                }
                let s = core::str::from_utf8(&b[start..j]).map_err(|_| Unsupported)?;
                out.push(Tok::Ident(s.into()));
                i = j + 1;
            }
            b'\'' => {
                // FHIRPath string escapes: \' \" \` \\ \/ \f \n \r \t \uXXXX
                let mut j = i + 1;
                let mut out_s = String::new();
                loop {
                    let Some(&c) = b.get(j) else { return Err(Unsupported) };
                    match c {
                        b'\'' => break,
                        b'\\' => {
                            let Some(&e) = b.get(j + 1) else { return Err(Unsupported) };
                            j += 2;
                            match e {
                                b'\'' | b'"' | b'`' | b'\\' | b'/' => out_s.push(e as char),
                                b'f' => out_s.push('\u{c}'),
                                b'n' => out_s.push('\n'),
                                b'r' => out_s.push('\r'),
                                b't' => out_s.push('\t'),
                                b'u' => {
                                    let hex = b.get(j..j + 4).ok_or(Unsupported)?;
                                    let mut v = 0u32;
                                    for &h in hex {
                                        v = v * 16 + (h as char).to_digit(16).ok_or(Unsupported)?;
                                    }
                                    out_s.push(char::from_u32(v).ok_or(Unsupported)?);
                                    j += 4;
                                }
                                _ => return Err(Unsupported),
                            }
                        }
                        _ => {
                            // copy one UTF-8 scalar
                            let len = match c {
                                0x00..=0x7F => 1,
                                0xC0..=0xDF => 2,
                                0xE0..=0xEF => 3,
                                _ => 4,
                            };
                            let s =
                                core::str::from_utf8(b.get(j..j + len).ok_or(Unsupported)?).map_err(|_| Unsupported)?;
                            out_s.push_str(s);
                            j += len;
                        }
                    }
                }
                out.push(Tok::Str(out_s));
                i = j + 1;
            }
            b'0'..=b'9' => {
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i < b.len() && b[i] == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                    return Err(Unsupported); // decimals: not in the core
                }
                let n =
                    core::str::from_utf8(&b[start..i]).ok().and_then(|s| s.parse::<i64>().ok()).ok_or(Unsupported)?;
                out.push(Tok::Int(n));
            }
            b'$' | b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = i;
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let s = core::str::from_utf8(&b[start..i]).map_err(|_| Unsupported)?;
                if s.starts_with('$') && s != "$this" {
                    return Err(Unsupported); // $index / $total: not in the core
                }
                out.push(Tok::Ident(s.into()));
            }
            b'%' => {
                let start = i + 1;
                let mut j = start;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if j == start {
                    return Err(Unsupported); // `%'quoted'` variables: not in the core
                }
                let s = core::str::from_utf8(&b[start..j]).map_err(|_| Unsupported)?;
                out.push(Tok::Var(s.into()));
                i = j;
            }
            _ => {
                out.push(Tok::Other);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Expression nesting bound (parens, call arguments): past this the parse
/// is `Unsupported` rather than a stack overflow.
const MAX_DEPTH: usize = 64;

struct P {
    toks: Vec<Tok>,
    i: usize,
    depth: usize,
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Ident(s)) if s == kw) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: Tok) -> Result<(), Unsupported> {
        if self.next() == Some(t) {
            Ok(())
        } else {
            Err(Unsupported)
        }
    }

    fn args(&mut self) -> Result<Vec<Expr>, Unsupported> {
        self.expect(Tok::LParen)?;
        let mut args = Vec::new();
        if !matches!(self.peek(), Some(Tok::RParen)) {
            loop {
                args.push(self.implies()?);
                if matches!(self.peek(), Some(Tok::Comma)) {
                    self.i += 1;
                } else {
                    break;
                }
            }
        }
        self.expect(Tok::RParen)?;
        Ok(args)
    }

    // Precedence, low to high (the spec's table, minus what's outside the
    // core): implies < or/xor < and < =/!= < comparison < +/- < * div mod
    // < unary - < path/call. `implies` is the entry of every nested
    // expression (parens, call arguments), so the depth bound lives here.
    fn implies(&mut self) -> Result<Expr, Unsupported> {
        if self.depth >= MAX_DEPTH {
            return Err(Unsupported);
        }
        self.depth += 1;
        let r = self.implies_inner();
        self.depth -= 1;
        r
    }

    fn implies_inner(&mut self) -> Result<Expr, Unsupported> {
        let mut l = self.or()?;
        while self.eat_kw("implies") {
            let r = self.or()?;
            l = Expr::Bin(Box::new(l), Op::Implies, Box::new(r));
        }
        Ok(l)
    }

    fn or(&mut self) -> Result<Expr, Unsupported> {
        let mut l = self.and()?;
        loop {
            let op = if self.eat_kw("or") {
                Op::Or
            } else if self.eat_kw("xor") {
                Op::Xor
            } else {
                return Ok(l);
            };
            let r = self.and()?;
            l = Expr::Bin(Box::new(l), op, Box::new(r));
        }
    }

    fn and(&mut self) -> Result<Expr, Unsupported> {
        let mut l = self.membership()?;
        while self.eat_kw("and") {
            let r = self.membership()?;
            l = Expr::Bin(Box::new(l), Op::And, Box::new(r));
        }
        Ok(l)
    }

    // `x in coll` / `coll contains x`
    fn membership(&mut self) -> Result<Expr, Unsupported> {
        let l = self.eq()?;
        if self.eat_kw("in") {
            let r = self.eq()?;
            return Ok(Expr::Bin(Box::new(l), Op::In, Box::new(r)));
        }
        if self.eat_kw("contains") {
            let r = self.eq()?;
            return Ok(Expr::Bin(Box::new(r), Op::In, Box::new(l)));
        }
        Ok(l)
    }

    fn eq(&mut self) -> Result<Expr, Unsupported> {
        let l = self.cmp()?;
        let op = match self.peek() {
            Some(Tok::Eq) => Op::Eq,
            Some(Tok::Ne) => Op::Ne,
            _ => return Ok(l),
        };
        self.i += 1;
        let r = self.cmp()?;
        Ok(Expr::Bin(Box::new(l), op, Box::new(r)))
    }

    fn cmp(&mut self) -> Result<Expr, Unsupported> {
        let l = self.union()?;
        let op = match self.peek() {
            Some(Tok::Lt) => Op::Lt,
            Some(Tok::Le) => Op::Le,
            Some(Tok::Gt) => Op::Gt,
            Some(Tok::Ge) => Op::Ge,
            _ => return Ok(l),
        };
        self.i += 1;
        let r = self.union()?;
        Ok(Expr::Bin(Box::new(l), op, Box::new(r)))
    }

    // `a | b` — union, distinct.
    fn union(&mut self) -> Result<Expr, Unsupported> {
        let mut l = self.type_expr()?;
        while matches!(self.peek(), Some(Tok::Pipe)) {
            self.i += 1;
            let r = self.type_expr()?;
            l = Expr::Bin(Box::new(l), Op::Union, Box::new(r));
        }
        Ok(l)
    }

    // `e is Type` / `e as Type` — binds tighter than comparison, looser
    // than arithmetic (the spec's table).
    fn type_expr(&mut self) -> Result<Expr, Unsupported> {
        let e = self.add()?;
        if self.eat_kw("is") {
            return Ok(Expr::Is(Box::new(e), self.type_specifier()?));
        }
        if self.eat_kw("as") {
            return Ok(Expr::As(Box::new(e), self.type_specifier()?));
        }
        Ok(e)
    }

    /// `dateTime`, `FHIR.dateTime`, `System.String` — kept as written; the
    /// namespace decides how the name is matched (see `type_matches`).
    fn type_specifier(&mut self) -> Result<String, Unsupported> {
        let Some(Tok::Ident(mut t)) = self.next() else {
            return Err(Unsupported);
        };
        while matches!(self.peek(), Some(Tok::Dot)) {
            self.i += 1;
            let Some(Tok::Ident(seg)) = self.next() else {
                return Err(Unsupported);
            };
            t.push('.');
            t.push_str(&seg);
        }
        Ok(t)
    }

    fn add(&mut self) -> Result<Expr, Unsupported> {
        let mut l = self.mul()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => Op::Add,
                Some(Tok::Minus) => Op::Sub,
                Some(Tok::Amp) => Op::Concat,
                _ => return Ok(l),
            };
            self.i += 1;
            let r = self.mul()?;
            l = Expr::Bin(Box::new(l), op, Box::new(r));
        }
    }

    fn mul(&mut self) -> Result<Expr, Unsupported> {
        let mut l = self.unary()?;
        loop {
            let op = if matches!(self.peek(), Some(Tok::Star)) {
                self.i += 1;
                Op::Mul
            } else if self.eat_kw("div") {
                Op::Div
            } else if self.eat_kw("mod") {
                Op::Mod
            } else {
                return Ok(l);
            };
            let r = self.unary()?;
            l = Expr::Bin(Box::new(l), op, Box::new(r));
        }
    }

    fn unary(&mut self) -> Result<Expr, Unsupported> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.i += 1;
                match self.next() {
                    Some(Tok::Int(n)) => Ok(Expr::Lit(Lit::Int(n.checked_neg().ok_or(Unsupported)?))),
                    _ => Err(Unsupported),
                }
            }
            Some(Tok::Plus) => {
                self.i += 1;
                match self.next() {
                    Some(Tok::Int(n)) => Ok(Expr::Lit(Lit::Int(n))),
                    _ => Err(Unsupported),
                }
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<Expr, Unsupported> {
        let mut e = self.primary()?;
        while matches!(self.peek(), Some(Tok::Dot)) {
            self.i += 1;
            let Some(Tok::Ident(name)) = self.next() else {
                return Err(Unsupported);
            };
            e = if matches!(self.peek(), Some(Tok::LParen)) {
                Expr::Call(Box::new(e), name, self.args()?)
            } else {
                Expr::Nav(Box::new(e), name)
            };
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, Unsupported> {
        match self.next() {
            Some(Tok::Str(s)) => Ok(Expr::Lit(Lit::Str(s))),
            Some(Tok::Int(n)) => Ok(Expr::Lit(Lit::Int(n))),
            Some(Tok::Var(v)) => Ok(Expr::Var(v)),
            Some(Tok::LParen) => {
                let e = self.implies()?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::Ident(s)) => match s.as_str() {
                "true" => Ok(Expr::Lit(Lit::Bool(true))),
                "false" => Ok(Expr::Lit(Lit::Bool(false))),
                "$this" => Ok(Expr::This),
                // A bare identifier navigates from `$this`; a bare call applies to it.
                _ if matches!(self.peek(), Some(Tok::LParen)) => Ok(Expr::Call(Box::new(Expr::This), s, self.args()?)),
                _ => Ok(Expr::Nav(Box::new(Expr::This), s)),
            },
            _ => Err(Unsupported),
        }
    }
}

/// Parse an expression, or `Unsupported` if it uses anything outside the core.
pub fn parse(src: &str) -> Result<Expr, Unsupported> {
    let toks = tokenize(src)?;
    let mut p = P { toks, i: 0, depth: 0 };
    let e = p.implies()?;
    if p.i != p.toks.len() {
        return Err(Unsupported);
    }
    Ok(e)
}

/// R4 constraint compatibility, verified against validator_cli 6.10.4:
/// bdl-8 guards optional fullUrl (FHIRPathExpressionFixer), and the old
/// vital-sign date-precision expression applies only to dateTime, not
/// effectivePeriod. Ordinary expression parsing remains literal.
pub(crate) fn parse_constraint(src: &str) -> Result<Expr, Unsupported> {
    parse(match src {
        "fullUrl.contains('/_history/').not()" => "fullUrl.exists() implies fullUrl.contains('/_history/').not()",
        "($this as dateTime).toString().length() >= 8" => "$this is dateTime implies $this.toString().length() >= 8",
        _ => src,
    })
}

/// One item of a FHIRPath collection.
#[derive(Clone)]
pub(crate) enum Item<D> {
    Node(D),
    Str(String),
    Int(i64),
    Bool(bool),
}

/// Every item a plain string (the result of `&`, `toString()`, a string
/// literal) — then equality is string equality and a sort will do.
fn all_strings<D: Document>(coll: &[Item<D>]) -> Option<Vec<&str>> {
    coll.iter().map(|it| if let Item::Str(s) = it { Some(s.as_str()) } else { None }).collect()
}

fn scalar_of<D: Document>(it: &Item<D>) -> Option<String> {
    match it {
        Item::Node(d) => d.as_scalar().map(String::from),
        Item::Str(s) => Some(s.clone()),
        Item::Int(n) => Some(format!("{n}")),
        Item::Bool(b) => Some(String::from(if *b { "true" } else { "false" })),
    }
}

/// A collection in boolean context: empty is unknown; a lone boolean is
/// itself; anything else present is true.
fn truth<D: Document>(c: &[Item<D>]) -> Result<Option<bool>, Unsupported> {
    Ok(match c {
        [] => None,
        [Item::Bool(b)] => Some(*b),
        [Item::Node(d)] => Some(d.as_bool().unwrap_or(true)),
        [_] => Some(true),
        // singleton evaluation of collections: more than one item is an error
        _ => return Err(Unsupported),
    })
}

/// What an expression is evaluated in besides `$this`: the type of the
/// invariant's element (for `is`/`as` on `$this`) and the resource root
/// (`%resource`). Inside `where`/`select`/…, `$this` is an arbitrary item
/// whose type is unknown, so `is`/`as` there are unsupported.
pub(crate) struct Env<'e, D> {
    pub this_type: Option<&'e str>,
    /// `%resource` — the resource the expression runs in.
    pub root: Option<&'e Item<D>>,
    /// The document `Bundle` that resource sits in, when it does: what
    /// `resolve()` looks references up in.
    pub bundle: Option<&'e Item<D>>,
}

impl<'e, D> Env<'e, D> {
    fn inner(&self) -> Env<'e, D> {
        Env { this_type: None, root: self.root, bundle: self.bundle }
    }
}

/// The target of `reference` inside the bundle / resource the expression
/// runs in: `#id` in `%resource.contained`; a Bundle entry by `fullUrl`, or
/// by `Type/id` against a `fullUrl` ending so or the entry's resource type
/// and id. A `/_history/version` suffix selects the matching `meta.versionId`.
/// `None` when it is not there — never fetched.
pub fn resolve_reference<D: Document>(reference: &str, resource: Option<&D>, bundle: Option<&D>) -> Option<D> {
    if let Some(local) = reference.strip_prefix('#') {
        return resource?.child_instances("contained").into_iter().find(|c| c.scalars("$this.id") == [local]);
    }
    let bundle = bundle?;
    let (identity, version) = match reference.rsplit_once("/_history/") {
        Some((identity, version)) if !version.is_empty() && !version.contains('/') => (identity, Some(version)),
        Some(_) => return None,
        None => (reference, None),
    };
    let relative = identity.split_once('/').filter(|(ty, id)| !ty.is_empty() && !id.contains('/'));
    for entry in bundle.child_instances("entry") {
        let full = entry.scalars("$this.fullUrl");
        let resource = entry.child_instances("resource").into_iter().next();
        if version.is_some_and(|v| resource.as_ref().is_none_or(|r| r.scalars("$this.meta.versionId") != [v])) {
            continue;
        }
        if full.contains(&identity) {
            return resource;
        }
        if let (Some((ty, id)), Some(res)) = (relative, resource.as_ref()) {
            let ends = full
                .iter()
                .any(|f| f.strip_suffix(id).and_then(|s| s.strip_suffix('/')).is_some_and(|s| s.ends_with(ty)));
            let same = res.scalars("$this.resourceType") == [ty] && res.scalars("$this.id") == [id];
            if ends || same {
                return resource;
            }
        }
    }
    None
}

/// `where(cond)`: each item becomes `$this` for its own test.
fn filter<D: Document>(coll: &[Item<D>], cond: &Expr, env: &Env<'_, D>) -> Result<Vec<Item<D>>, Unsupported> {
    let inner = env.inner();
    let mut out = Vec::new();
    for it in coll {
        if truth(&eval(cond, it, &inner)?)? == Some(true) {
            out.push(it.clone());
        }
    }
    Ok(out)
}

/// FHIRPath `=` on two single items: integers numerically, booleans,
/// dates / times temporally (empty when precisions or timezone presence
/// differ), text by value, complex nodes structurally; items of different
/// kinds are simply not equal. A decimal is outside the core.
fn equal<D: Document>(a: &Item<D>, b: &Item<D>) -> Result<Option<bool>, Unsupported> {
    if let (Some(x), Some(y)) = (num_of(a)?, num_of(b)?) {
        return Ok(Some(x == y));
    }
    let bool_of = |it: &Item<D>| match it {
        Item::Bool(b) => Some(*b),
        Item::Node(d) => d.as_bool(),
        _ => None,
    };
    if let (Some(x), Some(y)) = (bool_of(a), bool_of(b)) {
        return Ok(Some(x == y));
    }
    if is_text(a) && is_text(b) {
        let (x, y) = (text_of(a)?, text_of(b)?);
        return Ok(match (temporal::parse(&x), temporal::parse(&y)) {
            (Some(tx), Some(ty)) => temporal::compare(&tx, &ty).map(|o| o.is_eq()),
            _ => Some(x == y),
        });
    }
    if let (Item::Node(x), Item::Node(y)) = (a, b) {
        if x.as_scalar().is_none() && y.as_scalar().is_none() {
            return Ok(Some(x.same(y)));
        }
    }
    Ok(Some(false))
}

/// Equality as a plain yes/no (for `in`, `union`, `intersect`, `distinct`):
/// an undecidable comparison counts as not equal.
fn items_equal<D: Document>(a: &Item<D>, b: &Item<D>) -> bool {
    matches!(equal(a, b), Ok(Some(true)))
}

/// The FHIR type of an item: the profile's declared type for `$this` when
/// the engine supplied one, else what the document adapter knows, else
/// the system type of a literal. Unknown → unsupported, never guessed.
fn item_type<'a, D: Document>(it: &'a Item<D>, operand: &Expr, env: &Env<'a, D>) -> Result<&'a str, Unsupported> {
    if matches!(operand, Expr::This) {
        if let Some(t) = env.this_type {
            return Ok(t);
        }
    }
    match it {
        Item::Node(d) => d.fhir_type().ok_or(Unsupported),
        // a literal is a System value, not a FHIR element
        Item::Str(_) => Ok("System.String"),
        Item::Int(_) => Ok("System.Integer"),
        Item::Bool(_) => Ok("System.Boolean"),
    }
}

/// Is `spec` a type name at all — a FHIR type, an abstract base, or a
/// System type? An unknown name in `as` / `ofType` is an error, as the
/// reference engine has it, never an empty result.
fn known_type_name(ns: Option<&str>, spec: &str) -> bool {
    let system =
        matches!(spec, "String" | "Integer" | "Decimal" | "Boolean" | "Date" | "DateTime" | "Time" | "Quantity");
    match ns {
        Some("System") => system,
        Some("FHIR") => crate::typing::known_type(spec).is_some() || ABSTRACT.contains(&spec),
        _ => system || crate::typing::known_type(spec).is_some() || ABSTRACT.contains(&spec),
    }
}

const ABSTRACT: &[&str] = &[
    "Resource",
    "DomainResource",
    "Element",
    "BackboneElement",
    // the primitives: types of their own, with no children in the element table
    "boolean",
    "integer",
    "string",
    "decimal",
    "uri",
    "url",
    "canonical",
    "base64Binary",
    "instant",
    "date",
    "dateTime",
    "time",
    "code",
    "oid",
    "id",
    "markdown",
    "unsignedInt",
    "positiveInt",
    "uuid",
    "xhtml",
];

/// `as` / `ofType`: the item's type is exactly the one named — `code` is not
/// a `string` here, as the reference engine has it (unlike `is`).
fn type_is_exactly(actual: &str, ns: Option<&str>, spec: &str) -> bool {
    if let Some(system) = actual.strip_prefix("System.") {
        return ns != Some("FHIR") && system == spec;
    }
    ns != Some("System") && actual.strip_prefix("FHIR.").unwrap_or(actual) == spec
}

/// The type name in `as(canonical)` / `ofType(System.String)` — written as
/// a bare path, so it parses as navigation; read back here as (namespace, name).
fn type_arg(e: &Expr) -> Option<(Option<&str>, &str)> {
    match e {
        Expr::Nav(base, name) if matches!(**base, Expr::This) => Some((None, name)),
        Expr::Nav(base, name) => match &**base {
            Expr::Nav(inner, ns) if matches!(**inner, Expr::This) && (ns == "FHIR" || ns == "System") => {
                Some((Some(ns.as_str()), name))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Does a node of FHIR type `actual` satisfy the type specifier `spec`
/// (`dateTime`, `FHIR.dateTime`, `System.DateTime`)? See [`type_matches_ns`].
fn type_matches(actual: &str, spec: &str) -> bool {
    match spec.split_once('.') {
        Some((ns, name)) => type_matches_ns(actual, Some(ns), name),
        None => type_matches_ns(actual, None, spec),
    }
}

/// FHIRPath `is`, as the reference engine has it: a bare or `FHIR.` name is
/// a FHIR type, matched as written — the type itself or a supertype (`code`
/// is a `string`, `canonical` a `uri`, `Age` a `Quantity`, every resource a
/// `Resource` and, bar Bundle / Binary / Parameters, a `DomainResource`,
/// every datatype an `Element`). A `System.` name is the FHIRPath system
/// type a primitive maps to. Names are case-sensitive: `$this is DateTime`
/// on a FHIR `dateTime` is false, so US Core's `us-core-1` never fires —
/// there, or here.
fn type_matches_ns(actual: &str, ns: Option<&str>, spec: &str) -> bool {
    // a literal (`1`, `true`, `'x'`) is a System value: `1 is Integer` and
    // `1 is System.Integer` hold, `1 is FHIR.integer` does not
    if let Some(system) = actual.strip_prefix("System.") {
        return ns != Some("FHIR") && system == spec;
    }
    let actual = actual.strip_prefix("FHIR.").unwrap_or(actual);
    // a FHIR element is never a System type (`Patient.active is System.Boolean` is false)
    if ns == Some("System") {
        return false;
    }
    let is = |t: &str| t == spec;
    if is(actual) {
        return true;
    }
    let parents: &[&str] = match actual {
        "code" | "id" | "markdown" | "xhtml" => &["string"],
        "url" | "canonical" | "uuid" | "oid" => &["uri"],
        "positiveInt" | "unsignedInt" => &["integer"],
        "Age" | "Count" | "Distance" | "Duration" | "MoneyQuantity" | "SimpleQuantity" => &["Quantity", "Element"],
        "BackboneElement" => &["Element"],
        _ => &[],
    };
    if parents.iter().any(|p| is(p)) {
        return true;
    }
    if crate::typemap::RESOURCES.binary_search(&actual).is_ok() {
        return is("Resource") || (is("DomainResource") && !matches!(actual, "Bundle" | "Binary" | "Parameters"));
    }
    crate::typing::known_type(actual).is_some() && is("Element")
}

/// A collection that must be empty or a single item; more is unsupported
/// (the spec raises an error — we skip rather than guess).
fn single<D: Document>(coll: &[Item<D>]) -> Result<Option<&Item<D>>, Unsupported> {
    match coll {
        [] => Ok(None),
        [one] => Ok(Some(one)),
        _ => Err(Unsupported),
    }
}

/// The text of a string-valued item. Integers and booleans are not
/// strings: string functions on them are an error in the spec.
fn text_of<D: Document>(it: &Item<D>) -> Result<String, Unsupported> {
    match it {
        Item::Str(s) => Ok(s.clone()),
        Item::Node(d) if d.as_bool().is_none() && d.as_number().is_none() => {
            d.as_scalar().map(String::from).ok_or(Unsupported)
        }
        _ => Err(Unsupported),
    }
}

/// The integer an item denotes: an integer literal, or a JSON number node
/// that is an integer. A decimal is outside the core (`Err`); a non-number
/// is `Ok(None)`.
fn num_of<D: Document>(it: &Item<D>) -> Result<Option<i64>, Unsupported> {
    match it {
        Item::Int(n) => Ok(Some(*n)),
        Item::Node(d) => match d.as_number() {
            Some(text) => parse_int(text).map(Some).ok_or(Unsupported),
            None => Ok(None),
        },
        _ => Ok(None),
    }
}

/// Is this item text (a string literal or a JSON string node)?
fn is_text<D: Document>(it: &Item<D>) -> bool {
    match it {
        Item::Str(_) => true,
        Item::Node(d) => d.as_bool().is_none() && d.as_number().is_none() && d.as_scalar().is_some(),
        _ => false,
    }
}

fn int_of<D: Document>(it: &Item<D>) -> Result<i64, Unsupported> {
    match it {
        Item::Int(n) => Ok(*n),
        _ => Err(Unsupported),
    }
}

/// `toInteger()`: an integer stays; a boolean is 1/0; text matching
/// `[+-]?\d+` converts; anything else is empty.
fn to_integer<D: Document>(it: &Item<D>) -> Option<i64> {
    match it {
        Item::Int(n) => Some(*n),
        Item::Bool(b) => Some(*b as i64),
        Item::Node(d) if d.as_bool().is_some() => d.as_bool().map(|b| b as i64),
        Item::Node(d) => d.as_scalar().and_then(parse_int),
        Item::Str(s) => parse_int(s),
    }
}

fn parse_int(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `substring(start[, length])`, in characters; a start outside the
/// string (or a negative length) is empty, not an error.
fn substring(s: &str, start: i64, len: Option<i64>) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    if start < 0 || start as usize >= chars.len() {
        return None;
    }
    let start = start as usize;
    let end = match len {
        None => chars.len(),
        Some(l) if l < 0 => return None,
        Some(l) => (start + l as usize).min(chars.len()),
    };
    Some(chars[start..end].iter().collect())
}

fn arith(op: Op, x: i64, y: i64) -> Result<Option<i64>, Unsupported> {
    let r = match op {
        Op::Add => x.checked_add(y),
        Op::Sub => x.checked_sub(y),
        Op::Mul => x.checked_mul(y),
        // Division by zero is empty in the spec, not an error.
        Op::Div if y == 0 => return Ok(None),
        Op::Mod if y == 0 => return Ok(None),
        Op::Div => x.checked_div(y),
        Op::Mod => x.checked_rem(y),
        _ => return Err(Unsupported),
    };
    // Overflow: the spec's integers are 32-bit and overflow is an error.
    r.map(Some).ok_or(Unsupported)
}

/// `<` `<=` `>` `>=`: integers numerically; dates / times temporally
/// (empty when precisions or timezone presence differ); other text by
/// code point. Mixed kinds are outside the core.
fn compare<D: Document>(op: Op, a: &Item<D>, b: &Item<D>) -> Result<Option<bool>, Unsupported> {
    let ord = match (num_of(a)?, num_of(b)?) {
        (Some(x), Some(y)) => x.cmp(&y),
        (None, None) if is_text(a) && is_text(b) => {
            let (x, y) = (text_of(a)?, text_of(b)?);
            match (temporal::parse(&x), temporal::parse(&y)) {
                (Some(tx), Some(ty)) => match temporal::compare(&tx, &ty) {
                    Some(o) => o,
                    None => return Ok(None),
                },
                // Strings compare by code point, which is what `str`'s order is.
                _ => x.cmp(&y),
            }
        }
        _ => return Err(Unsupported),
    };
    Ok(Some(match op {
        Op::Lt => ord.is_lt(),
        Op::Le => ord.is_le(),
        Op::Gt => ord.is_gt(),
        Op::Ge => ord.is_ge(),
        _ => return Err(Unsupported),
    }))
}

fn logic(op: Op, a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match op {
        Op::And => match (a, b) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        Op::Or => match (a, b) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
        Op::Xor => match (a, b) {
            (Some(x), Some(y)) => Some(x != y),
            _ => None,
        },
        Op::Implies => match (a, b) {
            (Some(false), _) => Some(true),
            (Some(true), y) => y,
            (None, Some(true)) => Some(true),
            (None, _) => None,
        },
        _ => None,
    }
}

/// Evaluate with `ctx` as `$this` — a node, or a scalar item when inside
/// `select`/`where` over scalars (the Luhn invariants iterate digits).
fn eval<D: Document>(e: &Expr, ctx: &Item<D>, env: &Env<'_, D>) -> Result<Vec<Item<D>>, Unsupported> {
    Ok(match e {
        Expr::This => vec![ctx.clone()],
        Expr::Var(name) => match (name.as_str(), env.root) {
            ("resource", Some(root)) => vec![root.clone()],
            _ => return Err(Unsupported),
        },
        Expr::Is(operand, ty) => {
            let coll = eval(operand, ctx, env)?;
            let Some(it) = single(&coll)? else {
                return Ok(vec![]);
            };
            vec![Item::Bool(type_matches(item_type(it, operand, env)?, ty))]
        }
        Expr::As(operand, ty) => {
            let coll = eval(operand, ctx, env)?;
            let Some(it) = single(&coll)? else {
                return Ok(vec![]);
            };
            let (ns, spec) = ty.split_once('.').map(|(a, b)| (Some(a), b)).unwrap_or((None, ty.as_str()));
            if !known_type_name(ns, spec) {
                return Err(Unsupported);
            }
            if type_is_exactly(item_type(it, operand, env)?, ns, spec) {
                vec![it.clone()]
            } else {
                vec![]
            }
        }
        Expr::Lit(Lit::Str(s)) => vec![Item::Str(s.clone())],
        Expr::Lit(Lit::Int(n)) => vec![Item::Int(*n)],
        Expr::Lit(Lit::Bool(b)) => vec![Item::Bool(*b)],
        // A bare resource-type name (`QuestionnaireResponse.item`,
        // `Patient.name`) is `$this` when that is its type.
        Expr::Nav(base, name)
            if matches!(**base, Expr::This)
                && (env.this_type == Some(name.as_str())
                    || matches!(ctx, Item::Node(d) if d.fhir_type() == Some(name.as_str())
                        || d.scalars("$this.resourceType") == [name.as_str()])) =>
        {
            vec![ctx.clone()]
        }
        Expr::Nav(base, name) => {
            let mut out = Vec::new();
            for it in eval(base, ctx, env)? {
                let Item::Node(d) = it else {
                    return Err(Unsupported);
                };
                let mut found = d.child_instances(name);
                if found.is_empty() {
                    // Choice type: `value` reaches `valueQuantity` (typed by its key).
                    found = d.choice_children(name);
                }
                out.extend(found.into_iter().map(Item::Node));
            }
            out
        }
        Expr::Call(base, name, args) => {
            let coll = eval(base, ctx, env)?;
            match (name.as_str(), args.len()) {
                ("exists", 0) => vec![Item::Bool(!coll.is_empty())],
                ("exists", 1) => vec![Item::Bool(!filter(&coll, &args[0], env)?.is_empty())],
                ("empty", 0) => vec![Item::Bool(coll.is_empty())],
                ("not", 0) => match truth(&coll)? {
                    Some(b) => vec![Item::Bool(!b)],
                    None => vec![],
                },
                ("count", 0) => vec![Item::Int(coll.len() as i64)],
                ("first", 0) => coll.into_iter().take(1).collect(),
                // `resolve()`: references to what sits alongside — contained,
                // or in the document Bundle. Anything else cannot be fetched
                // here, and is not guessed.
                ("resolve", 0) => {
                    let (resource, bundle) = (
                        match env.root {
                            Some(Item::Node(d)) => Some(d),
                            _ => None,
                        },
                        match env.bundle {
                            Some(Item::Node(d)) => Some(d),
                            _ => None,
                        },
                    );
                    let mut out = Vec::new();
                    for it in &coll {
                        let reference = match it {
                            Item::Str(s) => s.clone(),
                            Item::Node(d) => match d.as_scalar() {
                                Some(s) => s.into(),
                                None => match d.scalars("$this.reference").first() {
                                    Some(s) => (*s).into(),
                                    None => continue,
                                },
                            },
                            _ => return Err(Unsupported),
                        };
                        match resolve_reference(&reference, resource, bundle) {
                            Some(target) => out.push(Item::Node(target)),
                            None if reference.starts_with('#') || bundle.is_some() => {}
                            None => return Err(Unsupported),
                        }
                    }
                    out
                }
                ("hasValue", 0) => vec![Item::Bool(match coll.as_slice() {
                    [Item::Node(d)] => d.as_scalar().is_some(),
                    [_] => true,
                    _ => false,
                })],
                ("where", 1) => filter(&coll, &args[0], env)?,
                ("last", 0) => coll.into_iter().last().into_iter().collect(),
                // `single()`: the one item, empty for none; more is an error in the spec
                ("single", 0) => single(&coll)?.cloned().into_iter().collect(),
                ("select", 1) => {
                    let inner = env.inner();
                    let mut out = Vec::new();
                    for it in &coll {
                        out.extend(eval(&args[0], it, &inner)?);
                    }
                    out
                }
                ("all", 1) => {
                    let inner = env.inner();
                    let mut all = true;
                    for it in &coll {
                        all &= truth(&eval(&args[0], it, &inner)?)? == Some(true);
                    }
                    vec![Item::Bool(all)]
                }
                // `intersect(other)`: the items also in `other` (FHIRPath equality).
                ("intersect", 1) => {
                    let other = eval(&args[0], ctx, env)?;
                    coll.into_iter().filter(|a| other.iter().any(|b| items_equal(a, b))).collect()
                }
                // `is(T)` as a function: the type test on a single item.
                ("is", 1) => {
                    let Some((ns, ty)) = type_arg(&args[0]) else {
                        return Err(Unsupported);
                    };
                    match single(&coll)? {
                        None => vec![],
                        Some(it) => vec![Item::Bool(type_matches_ns(item_type(it, &args[0], &env.inner())?, ns, ty))],
                    }
                }
                ("tail", 0) => coll.into_iter().skip(1).collect(),
                // `contains(substring)` on a string (the `contains` operator on
                // collections is parsed as `in`, reversed)
                ("contains", 1) => {
                    let sub = eval(&args[0], ctx, env)?;
                    match (single(&coll)?, single(&sub)?) {
                        (Some(s), Some(t)) if is_text(s) && is_text(t) => {
                            vec![Item::Bool(text_of(s)?.contains(&text_of(t)?))]
                        }
                        (None, _) | (_, None) => vec![],
                        _ => return Err(Unsupported),
                    }
                }
                // `as(T)` / `ofType(T)` as functions: the items of that type.
                // (`as` on a collection is treated as a filter, as the
                // reference engine does in dom-3.)
                ("as", 1) | ("ofType", 1) => {
                    let Some((ns, ty)) = type_arg(&args[0]) else {
                        return Err(Unsupported);
                    };
                    // an unknown type name is an error. (`as` on more than one
                    // item is one too by the official suite, but the reference
                    // validator's own dom-3 — `descendants().as(canonical)` —
                    // needs the filter reading, and that is the engine agreed with)
                    if !known_type_name(ns, ty) {
                        return Err(Unsupported);
                    }
                    let mut out = Vec::new();
                    for it in &coll {
                        if type_is_exactly(item_type(it, &args[0], &env.inner())?, ns, ty) {
                            out.push(it.clone());
                        }
                    }
                    out
                }
                ("descendants", 0) => {
                    let mut out = Vec::new();
                    let mut frontier: Vec<Item<D>> = coll;
                    while let Some(it) = frontier.pop() {
                        if let Item::Node(d) = it {
                            for c in d.all_children() {
                                out.push(Item::Node(c.clone()));
                                frontier.push(Item::Node(c));
                            }
                        }
                    }
                    out
                }
                // `repeat(projection)`: the projection applied to the input
                // and then to each new item, until nothing new appears.
                ("repeat", 1) => {
                    let inner = env.inner();
                    let mut out: Vec<Item<D>> = Vec::new();
                    let mut frontier = coll;
                    while let Some(it) = frontier.pop() {
                        for n in eval(&args[0], &it, &inner)? {
                            if !out.iter().any(|o| items_equal(o, &n)) {
                                out.push(n.clone());
                                frontier.push(n);
                            }
                        }
                    }
                    out
                }
                ("allTrue", 0) | ("anyTrue", 0) | ("allFalse", 0) | ("anyFalse", 0) => {
                    let mut bools = Vec::new();
                    for it in &coll {
                        match it {
                            Item::Bool(b) => bools.push(*b),
                            Item::Node(d) => bools.push(d.as_bool().ok_or(Unsupported)?),
                            _ => return Err(Unsupported),
                        }
                    }
                    vec![Item::Bool(match name.as_str() {
                        "allTrue" => bools.iter().all(|b| *b),
                        "anyTrue" => bools.iter().any(|b| *b),
                        "allFalse" => bools.iter().all(|b| !*b),
                        _ => bools.iter().any(|b| !*b),
                    })]
                }
                ("isDistinct", 0) => {
                    // a collection of plain strings (a bundle's fullUrls) is
                    // sorted once rather than compared pairwise
                    let distinct = match all_strings(&coll) {
                        Some(mut keys) => {
                            keys.sort_unstable();
                            keys.windows(2).all(|w| w[0] != w[1])
                        }
                        None => coll.iter().enumerate().all(|(i, a)| !coll[..i].iter().any(|b| items_equal(a, b))),
                    };
                    vec![Item::Bool(distinct)]
                }
                ("distinct", 0) => {
                    if let Some(keys) = all_strings(&coll) {
                        let mut seen: Vec<&str> = Vec::with_capacity(keys.len());
                        let mut keep = Vec::with_capacity(keys.len());
                        for (i, k) in keys.iter().enumerate() {
                            match seen.binary_search(k) {
                                Ok(_) => {}
                                Err(at) => {
                                    seen.insert(at, k);
                                    keep.push(i);
                                }
                            }
                        }
                        return Ok(coll
                            .into_iter()
                            .enumerate()
                            .filter(|(i, _)| keep.binary_search(i).is_ok())
                            .map(|(_, it)| it)
                            .collect());
                    }
                    let mut out: Vec<Item<D>> = Vec::new();
                    for it in coll {
                        if !out.iter().any(|o| items_equal(o, &it)) {
                            out.push(it);
                        }
                    }
                    out
                }
                // `trace(name[, projection])` logs in other engines; here it is the identity.
                ("trace", 1 | 2) => coll,
                ("matches", 1) => match single(&coll)? {
                    Some(it) => {
                        let s = text_of(it)?;
                        let pats = eval(&args[0], ctx, env)?;
                        let Some(pat) = single(&pats)? else {
                            return Ok(vec![]);
                        };
                        let re = crate::regex::Regex::new(&text_of(pat)?).map_err(|_| Unsupported)?;
                        vec![Item::Bool(re.is_match(&s))]
                    }
                    None => vec![],
                },
                // `extension(url)` — the extensions of each node with that URL.
                ("extension", 1) => {
                    let url = match single(&eval(&args[0], ctx, env)?)? {
                        Some(u) => text_of(u)?,
                        None => return Ok(vec![]),
                    };
                    let mut out = Vec::new();
                    for it in &coll {
                        let Item::Node(d) = it else {
                            return Err(Unsupported);
                        };
                        for ext in d.child_instances("extension") {
                            let matches =
                                ext.child_instances("url").iter().any(|u| u.as_scalar() == Some(url.as_str()));
                            if matches {
                                out.push(Item::Node(ext));
                            }
                        }
                    }
                    out
                }
                // `iif(cond, then[, else])` — the input item is `$this` for all three.
                ("iif", 2 | 3) => {
                    let Some(this) = single(&coll)? else {
                        return Ok(vec![]);
                    };
                    let inner = env.inner();
                    match truth(&eval(&args[0], this, &inner)?)? {
                        Some(true) => eval(&args[1], this, &inner)?,
                        _ if args.len() == 3 => eval(&args[2], this, &inner)?,
                        _ => vec![],
                    }
                }
                ("toInteger", 0) => match single(&coll)? {
                    Some(it) => to_integer(it).map(Item::Int).into_iter().collect(),
                    None => vec![],
                },
                ("toString", 0) => match single(&coll)? {
                    Some(it) => match scalar_of(it) {
                        Some(value) => vec![Item::Str(value)],
                        // A present primitive may have extensions and no value.
                        // Conversion then has no value, rather than using the
                        // carrier object as text or declaring it unsupported.
                        None if matches!(it, Item::Node(d) if d.fhir_type().is_some_and(|t| t.starts_with(|c: char| c.is_ascii_lowercase()))) =>
                        {
                            vec![]
                        }
                        None => return Err(Unsupported),
                    },
                    None => vec![],
                },
                ("length", 0) => match single(&coll)? {
                    Some(it) => vec![Item::Int(text_of(it)?.chars().count() as i64)],
                    None => vec![],
                },
                ("startsWith", 1) => match single(&coll)? {
                    Some(it) => {
                        let s = text_of(it)?;
                        match single(&eval(&args[0], ctx, env)?)? {
                            Some(p) => vec![Item::Bool(s.starts_with(text_of(p)?.as_str()))],
                            None => vec![],
                        }
                    }
                    None => vec![],
                },
                ("substring", 1 | 2) => match single(&coll)? {
                    Some(it) => {
                        let s = text_of(it)?;
                        let Some(start) = single(&eval(&args[0], ctx, env)?)?.map(int_of).transpose()? else {
                            return Ok(vec![]);
                        };
                        let len = match args.get(1) {
                            Some(a) => match single(&eval(a, ctx, env)?)? {
                                Some(n) => Some(int_of(n)?),
                                None => return Ok(vec![]),
                            },
                            None => None,
                        };
                        substring(&s, start, len).map(Item::Str).into_iter().collect()
                    }
                    None => vec![],
                },
                _ => return Err(Unsupported),
            }
        }
        Expr::Bin(l, op @ (Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod), r) => {
            let (a, b) = (eval(l, ctx, env)?, eval(r, ctx, env)?);
            let (Some(a), Some(b)) = (single(&a)?, single(&b)?) else {
                return Ok(vec![]);
            };
            match (num_of(a)?, num_of(b)?) {
                (Some(x), Some(y)) => arith(*op, x, y)?.map(Item::Int).into_iter().collect(),
                // `+` on strings concatenates.
                (None, None) if *op == Op::Add && is_text(a) && is_text(b) => {
                    vec![Item::Str(text_of(a)? + &text_of(b)?)]
                }
                _ => return Err(Unsupported),
            }
        }
        Expr::Bin(l, Op::Concat, r) => {
            let (a, b) = (eval(l, ctx, env)?, eval(r, ctx, env)?);
            let text = |coll: &[Item<D>]| -> Result<String, Unsupported> {
                match single(coll)? {
                    None => Ok(String::new()),
                    Some(it) if is_text(it) => text_of(it),
                    Some(_) => Err(Unsupported),
                }
            };
            vec![Item::Str(text(&a)? + &text(&b)?)]
        }
        Expr::Bin(l, Op::Union, r) => {
            let mut out = eval(l, ctx, env)?;
            for it in eval(r, ctx, env)? {
                if !out.iter().any(|o| items_equal(o, &it)) {
                    out.push(it);
                }
            }
            // the left side is made distinct too
            let mut distinct: Vec<Item<D>> = Vec::new();
            for it in out {
                if !distinct.iter().any(|o| items_equal(o, &it)) {
                    distinct.push(it);
                }
            }
            distinct
        }
        Expr::Bin(l, Op::In, r) => {
            let (a, b) = (eval(l, ctx, env)?, eval(r, ctx, env)?);
            let Some(a) = single(&a)? else {
                return Ok(vec![]);
            };
            vec![Item::Bool(b.iter().any(|x| items_equal(a, x)))]
        }
        Expr::Bin(l, op @ (Op::Lt | Op::Le | Op::Gt | Op::Ge), r) => {
            let (a, b) = (eval(l, ctx, env)?, eval(r, ctx, env)?);
            let (Some(a), Some(b)) = (single(&a)?, single(&b)?) else {
                return Ok(vec![]);
            };
            compare(*op, a, b)?.map(Item::Bool).into_iter().collect()
        }
        Expr::Bin(l, op @ (Op::Eq | Op::Ne), r) => {
            let (a, b) = (eval(l, ctx, env)?, eval(r, ctx, env)?);
            if a.is_empty() || b.is_empty() {
                return Ok(vec![]);
            }
            if a.len() != 1 || b.len() != 1 {
                return Err(Unsupported);
            }
            match equal(&a[0], &b[0])? {
                Some(eq) => vec![Item::Bool(if *op == Op::Eq { eq } else { !eq })],
                None => vec![],
            }
        }
        Expr::Bin(l, op, r) => {
            let a = truth(&eval(l, ctx, env)?)?;
            let b = truth(&eval(r, ctx, env)?)?;
            match logic(*op, a, b) {
                Some(v) => vec![Item::Bool(v)],
                None => vec![],
            }
        }
    })
}

/// The outcome of evaluating an invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    Unsupported,
}

/// Select extension-context nodes from the containing resource. This is a
/// node query, not a boolean invariant: a true predicate does not select an
/// element. Version-spanning definitions can name a root type absent in R4;
/// that root-only type filter cannot match this R4 resource.
pub(crate) fn context_nodes<D: Document>(expr: &Expr, resource: &D, bundle: Option<&D>) -> Result<Vec<D>, Unsupported> {
    if let Expr::Call(base, name, args) = expr {
        if matches!(**base, Expr::This) && name == "ofType" && args.len() == 1 {
            if let Some((ns, ty)) = type_arg(&args[0]) {
                if !known_type_name(ns, ty) {
                    return Ok(Vec::new());
                }
            }
        }
    }
    let root_item = Item::Node(resource.clone());
    let bundle_item = bundle.map(|d| Item::Node(d.clone()));
    let env = Env { this_type: resource.fhir_type(), root: Some(&root_item), bundle: bundle_item.as_ref() };
    Ok(eval(expr, &root_item, &env)?
        .into_iter()
        .filter_map(|item| match item {
            Item::Node(node) => Some(node),
            _ => None,
        })
        .collect())
}

/// Evaluate an invariant with `ctx` as `$this`. Empty is unknown and passes;
/// only a definite `false` fails.
pub fn check<D: Document>(expr: &Expr, ctx: &D) -> Verdict {
    check_with(expr, ctx, None, None)
}

/// [`check`] with the element's FHIR type (for `$this is …`) and the
/// resource root (for `%resource`).
pub fn check_with<D: Document>(expr: &Expr, ctx: &D, this_type: Option<&str>, root: Option<&D>) -> Verdict {
    check_in(expr, ctx, this_type, root, None)
}

/// [`check_with`] inside a document `Bundle`, which `resolve()` looks up in.
pub fn check_in<D: Document>(
    expr: &Expr,
    ctx: &D,
    this_type: Option<&str>,
    root: Option<&D>,
    bundle: Option<&D>,
) -> Verdict {
    check_result(expr, ctx, this_type, root, bundle, true)
}

/// Profile constraints require a non-empty true result, as validator_cli
/// does. The public expression helper keeps its documented empty/unknown
/// behavior; absence of an invariant's element is handled by the caller.
pub(crate) fn check_constraint_in<D: Document>(
    expr: &Expr,
    ctx: &D,
    this_type: Option<&str>,
    root: Option<&D>,
    bundle: Option<&D>,
) -> Verdict {
    check_result(expr, ctx, this_type, root, bundle, false)
}

fn check_result<D: Document>(
    expr: &Expr,
    ctx: &D,
    this_type: Option<&str>,
    root: Option<&D>,
    bundle: Option<&D>,
    empty_passes: bool,
) -> Verdict {
    let root_item = root.map(|r| Item::Node(r.clone()));
    let bundle_item = bundle.map(|b| Item::Node(b.clone()));
    let env = Env { this_type, root: root_item.as_ref(), bundle: bundle_item.as_ref() };
    match eval(expr, &Item::Node(ctx.clone()), &env).and_then(|coll| truth(&coll)) {
        Err(Unsupported) => Verdict::Unsupported,
        Ok(Some(false)) => Verdict::Fail,
        Ok(None) if !empty_passes => Verdict::Fail,
        Ok(_) => Verdict::Pass,
    }
}
