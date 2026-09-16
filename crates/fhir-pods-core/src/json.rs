//! JSON: `parse` builds a borrowed value tree (strings decoded only when they
//! carry escapes) — the correctness-first adapter behind the `Document` port.

use alloc::{borrow::Cow, string::String, vec::Vec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError {
    pub at: usize,
    pub msg: &'static str,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} at byte {}", self.msg, self.at)
    }
}

impl core::error::Error for ParseError {}

/// A borrowed JSON value tree. Numbers point into the input; strings do
/// too unless they carry escapes, in which case they are decoded into an
/// owned copy (`Cow`).
#[derive(Debug, Clone, PartialEq)]
pub enum Value<'a> {
    Null,
    Bool(bool),
    Number(&'a str),
    Str(Cow<'a, str>),
    Array(Vec<Value<'a>>),
    Object(Vec<(Cow<'a, str>, Value<'a>)>),
}

/// Parse one JSON document: a value with nothing but whitespace after it.
pub fn parse(input: &[u8]) -> Result<Value<'_>, ParseError> {
    let mut p = Parser { b: input, i: 0, depth: 0 };
    let v = p.parse_value()?;
    p.skip_ws();
    if p.i != input.len() {
        return Err(ParseError { at: p.i, msg: "trailing characters after the document" });
    }
    Ok(v)
}

/// Resolve a dotted path against a node. The first segment is the root's
/// name (the resource type, or `$this` for a sub-document) and is skipped.
/// Arrays expand; `[x]` matches a choice suffix (`value[x]` → `valueQuantity`).
pub fn resolve<'v, 'a>(root: &'v Value<'a>, path: &str) -> Vec<&'v Value<'a>> {
    let mut out = Vec::new();
    resolve_each(root, path, &mut |v| out.push(v));
    out
}

/// Visit every node `path` resolves to, depth-first, with no intermediate
/// collections. First segment (the root's name) is skipped.
pub fn resolve_each<'v, 'a>(root: &'v Value<'a>, path: &str, f: &mut dyn FnMut(&'v Value<'a>)) {
    match path.split_once('.') {
        None => f(root),
        Some((_, rest)) => walk(root, rest, f),
    }
}

fn walk<'v, 'a>(node: &'v Value<'a>, rest: &str, f: &mut dyn FnMut(&'v Value<'a>)) {
    match rest.split_once('.') {
        None => for_each_child(node, rest, f),
        Some((seg, tail)) => for_each_container(node, seg, &mut |c| walk(c, tail, f)),
    }
}

/// Visit the nodes `path` resolves to *as containers of further children*:
/// an object as is; a primitive stands in for its `_name` carrier (where its
/// `extension` and `id` live), and is skipped when it has none.
pub fn resolve_containers<'v, 'a>(root: &'v Value<'a>, path: &str, f: &mut dyn FnMut(&'v Value<'a>)) {
    match path.split_once('.') {
        None => f(root),
        Some((_, rest)) => match rest.rsplit_once('.') {
            None => for_each_container(root, rest, f),
            Some((head, last)) => walk(root, head, &mut |c| for_each_container(c, last, f)),
        },
    }
}

/// The children of `node` at `seg` (arrays expanded), in document order.
pub fn for_each_child<'v, 'a>(node: &'v Value<'a>, seg: &str, f: &mut dyn FnMut(&'v Value<'a>)) {
    for_each_child_ext(node, seg, &mut |v, _| f(v))
}

/// [`for_each_child`] for descending further: a primitive child yields its
/// `_name` carrier instead (paired by position in an array), or nothing.
pub fn for_each_container<'v, 'a>(node: &'v Value<'a>, seg: &str, f: &mut dyn FnMut(&'v Value<'a>)) {
    for_each_child_ext(node, seg, &mut |v, carrier| match v {
        Value::Object(_) | Value::Array(_) => f(v),
        _ => {
            if let Some(c) = carrier {
                f(c)
            }
        }
    })
}

fn is_primitive(v: &Value<'_>) -> bool {
    !matches!(v, Value::Object(_) | Value::Array(_))
}

/// The children of `node` at `seg`, each with its `_name` carrier element
/// when it is a primitive that has one. A child present only through the
/// carrier (`_name` with no `name`) is yielded as the carrier itself; the
/// carrier array's `null`s are placeholders, not elements.
pub fn for_each_child_ext<'v, 'a>(
    node: &'v Value<'a>,
    seg: &str,
    f: &mut dyn FnMut(&'v Value<'a>, Option<&'v Value<'a>>),
) {
    match node {
        Value::Object(members) => {
            let matches = |k: &str| match seg.strip_suffix("[x]") {
                Some(prefix) => {
                    k.len() > prefix.len() && k.starts_with(prefix) && k.as_bytes()[prefix.len()].is_ascii_uppercase()
                }
                None => k == seg,
            };
            let mut found = false;
            for (k, v) in members {
                if !matches(k) {
                    continue;
                }
                found = true;
                let carrier = members.iter().find(|(ck, _)| ck.strip_prefix('_') == Some(k.as_ref())).map(|(_, c)| c);
                match v {
                    Value::Array(items) => {
                        let carriers = match carrier {
                            Some(Value::Array(cs)) => Some(cs),
                            _ => None,
                        };
                        for (i, x) in items.iter().enumerate() {
                            let c = carriers.and_then(|cs| cs.get(i)).filter(|c| !matches!(c, Value::Null));
                            f(x, if is_primitive(x) { c } else { None });
                        }
                    }
                    x => f(x, if is_primitive(x) { carrier.filter(|c| matches!(c, Value::Object(_))) } else { None }),
                }
            }
            if !found {
                for (k, v) in members {
                    if let Some(bare) = k.strip_prefix('_') {
                        if matches(bare) {
                            expand(v, &mut |x| {
                                if !matches!(x, Value::Null) {
                                    f(x, None)
                                }
                            });
                        }
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                for_each_child_ext(item, seg, f);
            }
        }
        _ => {}
    }
}

fn expand<'v, 'a>(v: &'v Value<'a>, f: &mut dyn FnMut(&'v Value<'a>)) {
    match v {
        Value::Array(items) => {
            for x in items {
                f(x)
            }
        }
        other => f(other),
    }
}

/// Push the children of `node` matching `seg` (expanding arrays).
pub fn children<'v, 'a>(node: &'v Value<'a>, seg: &str, out: &mut Vec<&'v Value<'a>>) {
    for_each_child(node, seg, &mut |v| out.push(v));
}

/// Recursive descent must be bounded: past this, nesting is a `ParseError`
/// — never a stack overflow, which is an abort (not a panic) and fatal on
/// device. Real FHIR resources are ~10–20 deep.
const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn byte(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.byte(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), ParseError> {
        if self.byte() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(ParseError { at: self.i, msg: "unexpected byte" })
        }
    }

    /// A JSON string. Borrowed when it has no escapes; otherwise decoded
    /// (`\"`, `\\`, `\/`, `\b \f \n \r \t`, `\uXXXX` with surrogate
    /// pairs) into an owned copy. A lone surrogate becomes U+FFFD.
    fn string(&mut self) -> Result<Cow<'a, str>, ParseError> {
        self.expect(b'"')?;
        let b = self.b;
        let start = self.i;
        let mut owned: Option<String> = None;
        let mut run = start;
        loop {
            match b.get(self.i).copied() {
                None => return Err(ParseError { at: self.i, msg: "unterminated string" }),
                Some(b'"') => {
                    let tail = core::str::from_utf8(&b[run..self.i])
                        .map_err(|_| ParseError { at: run, msg: "invalid utf-8" })?;
                    self.i += 1;
                    return Ok(match owned {
                        None => Cow::Borrowed(tail),
                        Some(mut o) => {
                            o.push_str(tail);
                            Cow::Owned(o)
                        }
                    });
                }
                Some(b'\\') => {
                    let o = owned.get_or_insert_with(String::new);
                    o.push_str(
                        core::str::from_utf8(&b[run..self.i])
                            .map_err(|_| ParseError { at: run, msg: "invalid utf-8" })?,
                    );
                    let at = self.i;
                    let esc = b.get(self.i + 1).copied().ok_or(ParseError { at, msg: "unterminated string" })?;
                    self.i += 2;
                    match esc {
                        b'"' => o.push('"'),
                        b'\\' => o.push('\\'),
                        b'/' => o.push('/'),
                        b'b' => o.push('\u{8}'),
                        b'f' => o.push('\u{c}'),
                        b'n' => o.push('\n'),
                        b'r' => o.push('\r'),
                        b't' => o.push('\t'),
                        b'u' => {
                            let hi = self.hex4().ok_or(ParseError { at, msg: "bad \\u escape" })?;
                            let c = if (0xD800..0xDC00).contains(&hi) {
                                // a high surrogate: needs a \uDC00–\uDFFF partner
                                if b.get(self.i..self.i + 2) == Some(b"\\u") {
                                    let save = self.i;
                                    self.i += 2;
                                    match self.hex4() {
                                        Some(lo) if (0xDC00..0xE000).contains(&lo) => {
                                            char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                                        }
                                        _ => {
                                            self.i = save;
                                            None
                                        }
                                    }
                                } else {
                                    None
                                }
                            } else {
                                char::from_u32(hi)
                            };
                            o.push(c.unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(ParseError { at, msg: "bad escape" }),
                    }
                    run = self.i;
                }
                Some(_) => self.i += 1,
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let s = self.b.get(self.i..self.i + 4)?;
        let mut v = 0u32;
        for &c in s {
            v = v * 16 + (c as char).to_digit(16)?;
        }
        self.i += 4;
        Some(v)
    }

    fn literal(&mut self, lit: &[u8]) -> Result<(), ParseError> {
        if self.b.get(self.i..).is_some_and(|s| s.starts_with(lit)) {
            self.i += lit.len();
            Ok(())
        } else {
            Err(ParseError { at: self.i, msg: "bad literal" })
        }
    }

    fn parse_number(&mut self) -> Result<Value<'a>, ParseError> {
        let b = self.b;
        let start = self.i;
        while let Some(c) = self.byte() {
            if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.i += 1;
            } else {
                break;
            }
        }
        if self.i == start {
            return Err(ParseError { at: start, msg: "expected value" });
        }
        let s = core::str::from_utf8(&b[start..self.i]).map_err(|_| ParseError { at: start, msg: "invalid utf-8" })?;
        Ok(Value::Number(s))
    }

    fn parse_value(&mut self) -> Result<Value<'a>, ParseError> {
        self.skip_ws();
        match self.byte() {
            Some(c @ (b'{' | b'[')) => {
                if self.depth >= MAX_DEPTH {
                    return Err(ParseError { at: self.i, msg: "nesting too deep" });
                }
                self.depth += 1;
                let r = if c == b'{' { self.parse_object() } else { self.parse_array() };
                self.depth -= 1;
                r
            }
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') => {
                self.literal(b"true")?;
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.literal(b"false")?;
                Ok(Value::Bool(false))
            }
            Some(b'n') => {
                self.literal(b"null")?;
                Ok(Value::Null)
            }
            Some(_) => self.parse_number(),
            None => Err(ParseError { at: self.i, msg: "unexpected end" }),
        }
    }

    fn parse_object(&mut self) -> Result<Value<'a>, ParseError> {
        self.expect(b'{')?;
        let mut members = Vec::new();
        self.skip_ws();
        if self.byte() == Some(b'}') {
            self.i += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            let val = self.parse_value()?;
            members.push((key, val));
            self.skip_ws();
            match self.byte() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Object(members));
                }
                _ => return Err(ParseError { at: self.i, msg: "expected ',' or '}'" }),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Value<'a>, ParseError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.byte() == Some(b']') {
            self.i += 1;
            return Ok(Value::Array(items));
        }
        loop {
            let val = self.parse_value()?;
            items.push(val);
            self.skip_ws();
            match self.byte() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(ParseError { at: self.i, msg: "expected ',' or ']'" }),
            }
        }
    }
}
