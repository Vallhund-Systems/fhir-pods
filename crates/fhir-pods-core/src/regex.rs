//! A small regular-expression engine for FHIRPath `matches()`. Thompson-style
//! NFA simulation — linear in input × program, so hostile patterns and
//! inputs cannot blow up (no backtracking). Covers what invariants actually
//! use: literals, `.`, classes (`[0-9]`, `[^a-z]`, `\d \w \s`), anchors,
//! groups, alternation, and `* + ? {n} {n,} {n,m}` quantifiers (expanded,
//! bounded). Anything else (backreferences, lookaround, flags, `\b`,
//! unicode properties) is `Unsupported` — skipped, never guessed.

use alloc::{boxed::Box, vec::Vec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported;

impl core::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("outside the supported core")
    }
}

impl core::error::Error for Unsupported {}

#[derive(Debug, Clone)]
enum ClassItem {
    Ch(char),
    Range(char, char),
    Digit,
    Word,
    Space,
    NotDigit,
    NotWord,
    NotSpace,
}

#[derive(Clone)]
enum Ast {
    Char(char),
    Any,
    Class(bool, Vec<ClassItem>),
    Start,
    End,
    Group(Vec<Vec<Ast>>),
    Repeat(Box<Ast>, u32, Option<u32>),
}

/// Bounds on the compiled program, so a pathological pattern is refused
/// rather than expanded without limit.
const MAX_PROGRAM: usize = 4096;
const MAX_REPEAT: u32 = 256;

struct Parser<'a> {
    s: &'a [char],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn alternation(&mut self) -> Result<Vec<Vec<Ast>>, Unsupported> {
        if self.depth > 32 {
            return Err(Unsupported);
        }
        self.depth += 1;
        let mut alts = alloc::vec![Vec::new()];
        while let Some(c) = self.peek() {
            match c {
                ')' => break,
                '|' => {
                    self.i += 1;
                    alts.push(Vec::new());
                }
                _ => {
                    let atom = self.atom()?;
                    let atom = self.quantified(atom)?;
                    alts.last_mut().unwrap().push(atom);
                }
            }
        }
        self.depth -= 1;
        Ok(alts)
    }

    fn atom(&mut self) -> Result<Ast, Unsupported> {
        let c = self.peek().ok_or(Unsupported)?;
        self.i += 1;
        Ok(match c {
            '(' => {
                if self.peek() == Some('?') {
                    // only the non-capturing form; lookaround / flags / names are not supported
                    if self.s.get(self.i + 1) == Some(&':') {
                        self.i += 2;
                    } else {
                        return Err(Unsupported);
                    }
                }
                let alts = self.alternation()?;
                if self.peek() != Some(')') {
                    return Err(Unsupported);
                }
                self.i += 1;
                Ast::Group(alts)
            }
            '[' => self.class()?,
            '.' => Ast::Any,
            '^' => Ast::Start,
            '$' => Ast::End,
            '\\' => self.escape(false)?,
            '*' | '+' | '?' | '{' | ')' | '|' => return Err(Unsupported),
            c => Ast::Char(c),
        })
    }

    fn escape(&mut self, in_class: bool) -> Result<Ast, Unsupported> {
        let c = self.peek().ok_or(Unsupported)?;
        self.i += 1;
        let item = |it: ClassItem| Ast::Class(false, alloc::vec![it]);
        Ok(match c {
            'd' => item(ClassItem::Digit),
            'D' => item(ClassItem::NotDigit),
            'w' => item(ClassItem::Word),
            'W' => item(ClassItem::NotWord),
            's' => item(ClassItem::Space),
            'S' => item(ClassItem::NotSpace),
            'n' => Ast::Char('\n'),
            't' => Ast::Char('\t'),
            'r' => Ast::Char('\r'),
            c if c.is_ascii_alphanumeric() => return Err(Unsupported), // \b, \1, \p{..}, …
            c => {
                let _ = in_class;
                Ast::Char(c)
            }
        })
    }

    fn class(&mut self) -> Result<Ast, Unsupported> {
        let neg = self.peek() == Some('^');
        if neg {
            self.i += 1;
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek().ok_or(Unsupported)?;
            if c == ']' && !first {
                self.i += 1;
                break;
            }
            first = false;
            self.i += 1;
            let lo = if c == '\\' {
                match self.escape(true)? {
                    Ast::Char(ch) => ch,
                    Ast::Class(_, mut it) => {
                        items.push(it.remove(0));
                        continue;
                    }
                    _ => return Err(Unsupported),
                }
            } else {
                c
            };
            if self.peek() == Some('-') && self.s.get(self.i + 1).is_some_and(|&n| n != ']') {
                self.i += 1;
                let hi = self.peek().ok_or(Unsupported)?;
                self.i += 1;
                let hi = if hi == '\\' {
                    match self.escape(true)? {
                        Ast::Char(ch) => ch,
                        _ => return Err(Unsupported),
                    }
                } else {
                    hi
                };
                if lo > hi {
                    return Err(Unsupported);
                }
                items.push(ClassItem::Range(lo, hi));
            } else {
                items.push(ClassItem::Ch(lo));
            }
        }
        Ok(Ast::Class(neg, items))
    }

    fn quantified(&mut self, atom: Ast) -> Result<Ast, Unsupported> {
        let (min, max) = match self.peek() {
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('?') => (0, Some(1)),
            Some('{') => {
                self.i += 1;
                let min = self.number()?;
                let max = if self.peek() == Some(',') {
                    self.i += 1;
                    if self.peek() == Some('}') {
                        None
                    } else {
                        Some(self.number()?)
                    }
                } else {
                    Some(min)
                };
                if self.peek() != Some('}')
                    || max.is_some_and(|m| m < min)
                    || min > MAX_REPEAT
                    || max.is_some_and(|m| m > MAX_REPEAT)
                {
                    return Err(Unsupported);
                }
                (min, max)
            }
            _ => return Ok(atom),
        };
        self.i += 1;
        if matches!(self.peek(), Some('?') | Some('+')) {
            self.i += 1; // lazy / possessive: same language for a yes/no match
        }
        if matches!(atom, Ast::Start | Ast::End) {
            return Err(Unsupported);
        }
        Ok(Ast::Repeat(Box::new(atom), min, max))
    }

    fn number(&mut self) -> Result<u32, Unsupported> {
        let start = self.i;
        let mut n: u32 = 0;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            n = n.checked_mul(10).and_then(|n| n.checked_add(d)).ok_or(Unsupported)?;
            self.i += 1;
        }
        if self.i == start {
            return Err(Unsupported);
        }
        Ok(n)
    }
}

#[derive(Debug, Clone)]
enum Inst {
    Char(char),
    Any,
    Class(bool, Vec<ClassItem>),
    Start,
    End,
    Split(usize, usize),
    Jmp(usize),
    Match,
}

/// A compiled pattern.
#[derive(Debug)]
pub struct Regex {
    prog: Vec<Inst>,
}

impl Regex {
    pub fn new(pattern: &str) -> Result<Regex, Unsupported> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { s: &chars, i: 0, depth: 0 };
        let alts = p.alternation()?;
        if p.i != chars.len() {
            return Err(Unsupported); // an unbalanced ')'
        }
        let mut prog = Vec::new();
        emit(&Ast::Group(alts), &mut prog)?;
        prog.push(Inst::Match);
        Ok(Regex { prog })
    }

    /// Does the pattern match anywhere in `s` (FHIRPath `matches` is a
    /// search; anchor with `^…$` for a full match)?
    pub fn is_match(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        let n = self.prog.len();
        let mut cur: Vec<usize> = Vec::new();
        let mut next: Vec<usize> = Vec::new();
        let mut seen = alloc::vec![usize::MAX; n];
        for pos in 0..=chars.len() {
            // unanchored: a fresh thread may start at every position
            add_thread(&self.prog, 0, pos, &chars, &mut cur, &mut seen);
            next.clear();
            let mut matched = false;
            for &pc in &cur {
                match &self.prog[pc] {
                    Inst::Match => {
                        matched = true;
                        break;
                    }
                    Inst::Char(c) => {
                        if chars.get(pos) == Some(c) {
                            add_thread(&self.prog, pc + 1, pos + 1, &chars, &mut next, &mut seen);
                        }
                    }
                    Inst::Any => {
                        if pos < chars.len() {
                            add_thread(&self.prog, pc + 1, pos + 1, &chars, &mut next, &mut seen);
                        }
                    }
                    Inst::Class(neg, items) => {
                        if let Some(&c) = chars.get(pos) {
                            if class_matches(items, c) != *neg {
                                add_thread(&self.prog, pc + 1, pos + 1, &chars, &mut next, &mut seen);
                            }
                        }
                    }
                    Inst::Start | Inst::End | Inst::Split(..) | Inst::Jmp(_) => {}
                }
            }
            if matched {
                return true;
            }
            core::mem::swap(&mut cur, &mut next);
            if pos == chars.len() {
                break;
            }
        }
        false
    }
}

/// Follow epsilon edges from `pc` at input position `pos`, adding the
/// consuming (or Match) states reached. `seen` is keyed by position so a
/// state is added at most once per step: the linear-time guarantee.
fn add_thread(prog: &[Inst], pc: usize, pos: usize, chars: &[char], list: &mut Vec<usize>, seen: &mut [usize]) {
    let mut stack = alloc::vec![pc];
    while let Some(pc) = stack.pop() {
        if seen[pc] == pos {
            continue;
        }
        seen[pc] = pos;
        match &prog[pc] {
            Inst::Jmp(t) => stack.push(*t),
            Inst::Split(a, b) => {
                stack.push(*b);
                stack.push(*a);
            }
            Inst::Start => {
                if pos == 0 {
                    stack.push(pc + 1);
                }
            }
            Inst::End => {
                if pos == chars.len() {
                    stack.push(pc + 1);
                }
            }
            _ => list.push(pc),
        }
    }
}

fn class_matches(items: &[ClassItem], c: char) -> bool {
    items.iter().any(|it| match it {
        ClassItem::Ch(x) => *x == c,
        ClassItem::Range(lo, hi) => (*lo..=*hi).contains(&c),
        ClassItem::Digit => c.is_ascii_digit(),
        ClassItem::NotDigit => !c.is_ascii_digit(),
        ClassItem::Word => c.is_alphanumeric() || c == '_',
        ClassItem::NotWord => !(c.is_alphanumeric() || c == '_'),
        ClassItem::Space => c.is_whitespace(),
        ClassItem::NotSpace => !c.is_whitespace(),
    })
}

fn emit(ast: &Ast, prog: &mut Vec<Inst>) -> Result<(), Unsupported> {
    if prog.len() > MAX_PROGRAM {
        return Err(Unsupported);
    }
    match ast {
        Ast::Char(c) => prog.push(Inst::Char(*c)),
        Ast::Any => prog.push(Inst::Any),
        Ast::Class(neg, items) => prog.push(Inst::Class(*neg, items.clone())),
        Ast::Start => prog.push(Inst::Start),
        Ast::End => prog.push(Inst::End),
        Ast::Group(alts) => {
            // split → alt1 → jmp end; split → alt2 → jmp end; … lastalt
            let mut jumps = Vec::new();
            for (i, alt) in alts.iter().enumerate() {
                let last = i + 1 == alts.len();
                let split_at = prog.len();
                if !last {
                    prog.push(Inst::Split(split_at + 1, 0)); // patched below
                }
                for a in alt {
                    emit(a, prog)?;
                }
                if !last {
                    jumps.push(prog.len());
                    prog.push(Inst::Jmp(0)); // patched below
                    let next = prog.len();
                    prog[split_at] = Inst::Split(split_at + 1, next);
                }
            }
            let end = prog.len();
            for j in jumps {
                prog[j] = Inst::Jmp(end);
            }
        }
        Ast::Repeat(inner, min, max) => {
            for _ in 0..*min {
                emit(inner, prog)?;
            }
            match max {
                None => {
                    // loop: split(body, exit); body; jmp split
                    let split_at = prog.len();
                    prog.push(Inst::Split(split_at + 1, 0));
                    emit(inner, prog)?;
                    prog.push(Inst::Jmp(split_at));
                    let exit = prog.len();
                    prog[split_at] = Inst::Split(split_at + 1, exit);
                }
                Some(max) => {
                    // each optional copy: split(body, exit)
                    let mut splits = Vec::new();
                    for _ in *min..*max {
                        splits.push(prog.len());
                        prog.push(Inst::Split(prog.len() + 1, 0));
                        emit(inner, prog)?;
                    }
                    let exit = prog.len();
                    for s in splits {
                        prog[s] = Inst::Split(s + 1, exit);
                    }
                }
            }
        }
    }
    if prog.len() > MAX_PROGRAM {
        return Err(Unsupported);
    }
    Ok(())
}
