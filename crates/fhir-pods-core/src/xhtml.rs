//! The narrative's XHTML, checked as the reference validator checks it
//! (`txt-1`, through FHIRPath's `htmlChecks()`, and the parser's own
//! findings): well-formed XML; a `div` root, every element in the XHTML
//! namespace; only the elements and attributes the narrative allows; the
//! content model (a `li` in a list, a `td` in a row, no text directly in
//! a table, no block inside a paragraph or a text element); XML's own
//! entities only; the hyperlink rules. Every rule here was probed against
//! validator_cli 6.10.4 — a narrative is judged as it judges it, not as
//! the HTML spec would — and its oddities are kept on purpose: a
//! `javascript:` scheme is active content but `JavaScript:` is not; a
//! numeric reference that does not parse as a Java `int` passes; an
//! element left open at the end of the text passes, and so does a start
//! tag the text ends inside (the element keeps the attributes read so
//! far) — a *closing* tag the text ends inside does not; text or a second
//! root after the root is ignored; an image's source is not held to the
//! hyperlink schemes at all, so `javascript:` in an `img src` passes
//! unless the host asks for external narrative references to be refused.
//!
//! `no_std`: a hand tokenizer over the bytes; nothing is allocated beyond
//! the findings, the element stack and the link tables.

use alloc::borrow::ToOwned;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const XHTML: &str = "http://www.w3.org/1999/xhtml";

/// What the check found: the reference validator's message, and whether
/// `txt-1` fails with it — it does for a parse failure, a disallowed
/// element and a disallowed attribute; a namespace, root, content-model,
/// link or entity finding stands on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub message: String,
    pub txt1: bool,
}

/// One pass over a narrative: the findings (one per distinct message, as
/// the reference validator reports them), and whether it carries content
/// — decoded text beyond whitespace, or an image, anywhere under the root;
/// never CDATA, never a second root. A parse failure has no content.
pub struct Inspection {
    pub findings: Vec<Finding>,
    pub has_text: bool,
}

/// The reference validator's own nesting limit (probed: 501 open
/// elements under the root pass, the 502nd fails).
const MAX_DEPTH: usize = 500;

/// How many block structures a text element's finding names — the pod's
/// own bound, so a narrative cannot grow that list without limit (every
/// block under every text-element ancestor is a path of its own).
const MAX_BLOCKS: usize = 64;

/// The elements a narrative may carry (probed element by element).
const ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "area",
    "b",
    "bdo",
    "big",
    "blockquote",
    "br",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "kbd",
    "li",
    "map",
    "ol",
    "p",
    "pre",
    "q",
    "samp",
    "small",
    "span",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "tt",
    "ul",
    "var",
];

/// Where an element must sit (parent names).
const MUST_BE_IN: &[(&str, &[&str])] = &[
    ("caption", &["table"]),
    ("col", &["colgroup", "table"]),
    ("colgroup", &["table"]),
    ("dd", &["dl"]),
    ("dt", &["dl"]),
    ("li", &["ul", "ol"]),
    ("tbody", &["table"]),
    ("td", &["tr", "thead", "tfoot"]),
    ("tfoot", &["table"]),
    ("th", &["tr", "thead", "tfoot"]),
    ("thead", &["table"]),
    ("tr", &["table", "thead", "tfoot", "tbody"]),
];

/// Elements with a content model of their own: what they may hold; text
/// and anything else is "cannot have the following children".
const CHILDREN: &[(&str, &[&str])] = &[
    ("dl", &["dt", "dd"]),
    ("map", &["area"]),
    ("ol", &["li"]),
    ("table", &["caption", "colgroup", "col", "thead", "tfoot", "tbody", "tr"]),
    ("tbody", &["tr"]),
    ("tfoot", &["tr"]),
    ("thead", &["tr"]),
    ("tr", &["td", "th"]),
    ("ul", &["li"]),
];

/// The "paragraph/text elements": a block structure anywhere inside one
/// is reported against it (a `li`, `td`, `th`, `blockquote` or `div` is
/// not one).
const TEXT_ELEMENTS: &[&str] = &[
    "a", "abbr", "acronym", "address", "b", "bdo", "big", "caption", "cite", "code", "dd", "dfn", "dt", "em", "h1",
    "h2", "h3", "h4", "h5", "h6", "i", "kbd", "p", "pre", "q", "samp", "small", "span", "strong", "sub", "sup", "tt",
    "var",
];

/// The block structures a text element cannot hold.
const BLOCK: &[&str] = &["div", "ol", "pre", "table", "ul"];

/// What is an invalid element name inside a paragraph (at any depth).
const PARAGRAPH_INVALID: &[&str] = &["blockquote", "div", "ol", "p", "table", "ul"];

/// Named references the reference validator knows from HTML and refuses
/// in a narrative ("Invalid entity"); any other name is a parse failure.
const HTML_ENTITIES: &[&str] = &include!("xhtml_entities.rs");

/// The characters a URL may not carry.
const URL_INVALID: &[char] = &[' ', '\t', '\n', '\r', '"', '<', '>', '\\', '^', '`', '{', '}'];

/// `label` is the narrative's path in the resource (`Patient.text.div`):
/// an image's finding hangs off it. `known` are the ids a `#fragment`
/// resolves to beyond the narrative's own: the resource's id and its
/// contained resources' ids (probed). `refuse_external` is the host's
/// `externalNarrativeLinks` option: with it, a reference that is neither a
/// `#fragment` nor a self-contained `data:` URL is a finding of its own —
/// the oracle has no such rule, which is why it is the host's to ask for.
pub fn inspect(div: &str, label: &str, known: &[&str], refuse_external: bool) -> Inspection {
    let mut out = Vec::new();
    let mut p = Parser {
        s: div.as_bytes(),
        i: 0,
        out: &mut out,
        stack: Vec::new(),
        open: Vec::new(),
        ids: BTreeSet::new(),
        names: BTreeSet::new(),
        frags: Vec::new(),
        ns_decls: Vec::new(),
        text: false,
        links_open: 0,
        label,
        fatal: false,
        refuse_external,
    };
    p.document();
    let fatal = p.fatal;
    let frags = core::mem::take(&mut p.frags);
    let ids = core::mem::take(&mut p.ids);
    let names = core::mem::take(&mut p.names);
    let text = p.text && !fatal;
    if !fatal {
        for (frag, at, text, image) in frags {
            if !(ids.contains(&frag) || known.contains(&frag.as_str()) || (!image && names.contains(&frag))) {
                let message = if image {
                    format!("Image source '#{frag}' at '{at}' does not resolve")
                } else {
                    format!("Hyperlink '#{frag}' at '{at}' for '{text}' does not resolve")
                };
                out.push(Finding { message, txt1: false });
            }
        }
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    out.retain(|f| seen.insert(f.message.clone()));
    Inspection { findings: out, has_text: text }
}

/// The findings alone (`htmlChecks()`) — the invariant's own question, so
/// always the oracle's rules: a host option never moves `txt-1`.
pub fn check(div: &str) -> Vec<Finding> {
    inspect(div, "div", &[], false).findings
}

/// The fragments the narrative links to (`<a href="#id">`, `<img
/// src="#id">`), decoded: a contained resource one names is referenced
/// (dom-3), as the reference validator has it.
pub fn fragments(div: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut p = Parser {
        s: div.as_bytes(),
        i: 0,
        out: &mut out,
        stack: Vec::new(),
        open: Vec::new(),
        ids: BTreeSet::new(),
        names: BTreeSet::new(),
        frags: Vec::new(),
        ns_decls: Vec::new(),
        text: false,
        links_open: 0,
        label: "",
        fatal: false,
        refuse_external: false,
    };
    p.document();
    if p.fatal {
        return Vec::new();
    }
    p.frags.into_iter().map(|(frag, _, _, _)| frag).collect()
}

/// What an open element accumulates until it closes.
#[derive(Default)]
struct Open {
    /// the name as written, which the closing tag must match
    qname: String,
    /// where this element's namespace declarations start in `ns_decls`
    mark: usize,
    /// block structures found inside a text element (relative paths)
    blocks: Vec<String>,
    /// a hyperlink: its href and path, judged with its text once closed
    link: Option<(String, String)>,
    /// the hyperlink's text: decoded text and image alts under it
    text: String,
}

struct Parser<'a, 'o> {
    s: &'a [u8],
    i: usize,
    out: &'o mut Vec<Finding>,
    /// open elements, local names
    stack: Vec<String>,
    open: Vec<Open>,
    /// `id` values, which a hyperlink or an image fragment resolves to
    ids: BTreeSet<String>,
    /// `a name` values, which only a hyperlink fragment resolves to
    names: BTreeSet<String>,
    /// (fragment, path, link text, is an image) of every `#` link, resolved at the end
    frags: Vec<(String, String, String, bool)>,
    /// namespace declarations in scope (prefix, or None for the default)
    ns_decls: Vec<(Option<String>, String)>,
    text: bool,
    /// how many open elements carry a hyperlink (so text need not look)
    links_open: usize,
    label: &'a str,
    fatal: bool,
    /// The host refuses a narrative reference out of the resource.
    refuse_external: bool,
}

impl Parser<'_, '_> {
    /// A parse failure: the reference validator has no document then —
    /// only the failure is reported, and the narrative has no content.
    fn fail(&mut self, message: String) -> bool {
        self.out.clear();
        self.out.push(Finding { message, txt1: true });
        self.fatal = true;
        false
    }

    fn peek(&self, k: usize) -> Option<u8> {
        self.s.get(self.i + k).copied()
    }

    fn starts(&self, lit: &str) -> bool {
        self.s.get(self.i..).is_some_and(|r| r.starts_with(lit.as_bytes()))
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(0), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    fn path(&self) -> String {
        self.stack.join("/")
    }

    /// `lit` at or after `from`.
    fn find_from(&self, from: usize, lit: &str) -> Option<usize> {
        let b = lit.as_bytes();
        (from..self.s.len().saturating_sub(b.len().saturating_sub(1))).find(|&j| self.s[j..].starts_with(b))
    }

    fn document(&mut self) {
        if self.starts("\u{feff}") {
            self.i += 3;
        }
        // prolog: declaration, comments, doctype, whitespace
        loop {
            self.skip_ws();
            if self.starts("<?") {
                match self.find_from(self.i + 2, "?>") {
                    Some(j) => self.i = j + 2,
                    None => {
                        self.fail(
                            "Error parsing XHTML: Malformed XHTML: unterminated processing instruction".to_owned(),
                        );
                        return;
                    }
                }
            } else if self.starts("<!--") {
                // `<!--->` is a whole comment to the reference parser; `<!-->` is not
                match self.find_from(self.i + 3, "-->") {
                    Some(j) => self.i = j + 3,
                    None => {
                        self.fail("Error parsing XHTML: Unexpected termination of html source".to_owned());
                        return;
                    }
                }
            } else if self.starts("<!DOCTYPE") || self.starts("<!doctype") {
                // to the first `>` — an internal subset is not understood (probed)
                match self.find_from(self.i, ">") {
                    Some(j) => self.i = j + 1,
                    None => {
                        self.fail("Error parsing XHTML: Malformed XHTML: unterminated DOCTYPE".to_owned());
                        return;
                    }
                }
            } else {
                break;
            }
        }
        if self.peek(0) != Some(b'<') {
            let found: String = core::str::from_utf8(&self.s[self.i..]).unwrap_or("").chars().take(1).collect();
            self.fail(format!("Error parsing XHTML: Unable to Parse HTML - does not start with tag. Found '{found}'"));
            return;
        }
        if !self.open_element(true) {
            return;
        }
        // the content, with an explicit stack: nothing recurses, so the
        // reference validator's own nesting limit is reachable on any stack
        loop {
            // the root closed; whatever follows it — text, a second root — is ignored (probed)
            if self.stack.is_empty() {
                return;
            }
            if self.i >= self.s.len() {
                // whatever is still open at the end is taken as closed (probed)
                while !self.stack.is_empty() {
                    self.close();
                }
                return;
            }
            if self.starts("</") {
                let j = self.i + 2;
                let mut k = j;
                while k < self.s.len() && self.s[k] != b'>' {
                    k += 1;
                }
                if k >= self.s.len() {
                    // the text ends inside a closing tag: the one truncation
                    // the reference parser refuses (probed — a start tag it
                    // ends inside, and an element left open, both pass)
                    self.fail("Error parsing XHTML: Unexpected termination of html source".to_owned());
                    return;
                }
                let closing = core::str::from_utf8(&self.s[j..k]).unwrap_or("").trim().to_owned();
                let qname = self.open.last().map(|o| o.qname.clone()).unwrap_or_default();
                if closing != qname {
                    self.fail(format!(
                        "Error parsing XHTML: Malformed XHTML: Found \"</{closing}>\" expecting \"</{qname}>\""
                    ));
                    return;
                }
                self.i = (k + 1).min(self.s.len());
                self.close();
                continue;
            }
            if self.starts("<!--") {
                match self.find_from(self.i + 3, "-->") {
                    Some(j) => self.i = j + 3,
                    None => {
                        self.fail("Error parsing XHTML: Unexpected termination of html source".to_owned());
                        return;
                    }
                }
                continue;
            }
            if self.starts("<![CDATA[") {
                match self.find_from(self.i + 9, "]]>") {
                    Some(j) => self.i = j + 3, // CDATA is not text to the reference validator
                    None => {
                        self.fail("Error parsing XHTML: Malformed XHTML: unterminated CDATA".to_owned());
                        return;
                    }
                }
                continue;
            }
            if self.starts("<?") {
                match self.find_from(self.i + 2, "?>") {
                    Some(j) => self.i = j + 2,
                    None => {
                        self.fail(
                            "Error parsing XHTML: Malformed XHTML: unterminated processing instruction".to_owned(),
                        );
                        return;
                    }
                }
                continue;
            }
            if self.peek(0) == Some(b'<') {
                if !self.open_element(false) {
                    return;
                }
                continue;
            }
            // text
            let t0 = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'<' {
                self.i += 1;
            }
            let raw = core::str::from_utf8(&self.s[t0..self.i]).unwrap_or("").to_owned();
            let Some(decoded) = self.decode(&raw) else { return };
            // content is what is left after a Java trim (a non-breaking
            // space counts; an invalid entity stays as written and counts)
            if decoded.chars().any(|c| c > ' ') {
                self.text = true;
                let local = self.stack.last().cloned().unwrap_or_default();
                if CHILDREN.iter().any(|(e, _)| *e == local) {
                    let at = self.path();
                    self.out.push(Finding {
                        message: format!("Elements of type {local} at {at} cannot have the following children: @text"),
                        txt1: false,
                    });
                }
            }
            self.link_text(&decoded);
        }
    }

    /// One start tag at `self.i` (which is `<`): the element is judged and
    /// pushed (and popped again when it closes itself); `root` is whether
    /// it is the document root. Returns false on a parse failure.
    fn open_element(&mut self, root: bool) -> bool {
        if self.stack.len() > MAX_DEPTH + 1 {
            return self.fail(format!("Error parsing XHTML: XHTML nesting depth exceeds maximum of {MAX_DEPTH}"));
        }
        self.i += 1; // <
        let name_start = self.i;
        while self.peek(0).is_some_and(is_name_byte) {
            self.i += 1;
        }
        let qname = core::str::from_utf8(&self.s[name_start..self.i]).unwrap_or("").to_owned();
        if qname.is_empty() {
            let parent = self.stack.last().cloned().unwrap_or_default();
            return self
                .fail(format!("Error parsing XHTML: Unable to Parse HTML - node '{parent}' has unexpected content"));
        }
        let (prefix, local) = match qname.split_once(':') {
            Some((p, l)) => (Some(p.to_owned()), l.to_owned()),
            None => (None, qname.clone()),
        };
        // attributes
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut self_closing = false;
        // the text ended inside this start tag: the reference parser keeps
        // the element with the attributes it has read and ends the document
        // there, so the element is still judged (probed)
        let mut truncated = false;
        loop {
            self.skip_ws();
            match self.peek(0) {
                Some(b'>') => {
                    self.i += 1;
                    break;
                }
                Some(b'/') if self.peek(1) == Some(b'>') => {
                    self.i += 2;
                    self_closing = true;
                    break;
                }
                Some(c) if is_name_byte(c) => {
                    let a0 = self.i;
                    while self.peek(0).is_some_and(is_name_byte) {
                        self.i += 1;
                    }
                    let aname = core::str::from_utf8(&self.s[a0..self.i]).unwrap_or("").to_owned();
                    self.skip_ws();
                    // a bare attribute (`hidden`) is taken, and then judged by name
                    if self.peek(0) != Some(b'=') {
                        attrs.push((aname, String::new()));
                        continue;
                    }
                    self.i += 1;
                    self.skip_ws();
                    // an unquoted value and a repeated attribute are taken (probed)
                    let raw = match self.peek(0) {
                        Some(quote @ (b'"' | b'\'')) => {
                            self.i += 1;
                            let v0 = self.i;
                            // A start tag ends at the first `>`, quoted or
                            // not — the reference parser does not look for
                            // the matching quote first (probed: an `img`
                            // whose `src` carries a `>` is left open, and
                            // `</div>` then meets `</img>`). So the value is
                            // what was read, the `>` ends the tag, and what
                            // follows it is text. A value that reaches the
                            // end of the narrative instead truncates the
                            // document, which is the one case the parser
                            // tolerates there (both in
                            // `conformance/narrative/`).
                            while self.peek(0).is_some_and(|c| c != quote && c != b'>') {
                                self.i += 1;
                            }
                            let v = core::str::from_utf8(&self.s[v0..self.i]).unwrap_or("").to_owned();
                            match self.peek(0) {
                                Some(c) if c == quote => self.i += 1,
                                Some(_) => {} // the `>`: the loop's next pass ends the tag
                                None => truncated = true,
                            }
                            v
                        }
                        _ => {
                            let v0 = self.i;
                            while self.peek(0).is_some_and(|c| !c.is_ascii_whitespace() && c != b'>' && c != b'/') {
                                self.i += 1;
                            }
                            core::str::from_utf8(&self.s[v0..self.i]).unwrap_or("").to_owned()
                        }
                    };
                    let Some(value) = self.decode(&raw) else { return false };
                    attrs.push((aname, value));
                    if truncated {
                        break;
                    }
                }
                None => {
                    truncated = true;
                    break;
                }
                // a `/` no `>` follows is the one truncation the reference
                // parser refuses; anything else here is an unreadable
                // attribute (both probed)
                Some(b'/') => {
                    return self.fail(format!("Error parsing XHTML: unexpected non-end of element null::{qname}"));
                }
                Some(_) => {
                    return self.fail(format!("Error parsing XHTML: Unable to read attribute on <{qname}>"));
                }
            }
        }
        // the namespace: declared here or on an ancestor; a prefix nobody
        // declared is the reference parser's error namespace
        let mark = self.ns_decls.len();
        for (k, v) in &attrs {
            if k == "xmlns" {
                self.ns_decls.push((None, v.clone()));
            } else if let Some(p) = k.strip_prefix("xmlns:") {
                self.ns_decls.push((Some(p.to_owned()), v.clone()));
            }
        }
        let ns = self
            .ns_decls
            .iter()
            .rev()
            .find(|(p, _)| *p == prefix)
            .map(|(_, v)| v.clone())
            .or_else(|| prefix.as_ref().map(|_| "http://error/undefined-namespace".to_owned()));
        if root && local != "div" {
            self.out.push(Finding {
                message: format!("Wrong name on the XHTML ('{local}') - must start with div"),
                txt1: false,
            });
        }
        if ns.as_deref() != Some(XHTML) {
            self.out.push(Finding {
                message: format!(
                    "Wrong namespace on the XHTML ('{}', should be '{XHTML}')",
                    ns.as_deref().unwrap_or("null")
                ),
                txt1: false,
            });
        }
        if !ELEMENTS.contains(&local.as_str()) {
            self.out.push(Finding { message: format!("Invalid element name in the XHTML ('{local}')"), txt1: true });
        }
        if let Some(parent) = self.stack.last().cloned() {
            if let Some((_, children)) = CHILDREN.iter().find(|(e, _)| *e == parent) {
                if !children.contains(&local.as_str()) {
                    self.out.push(Finding {
                        message: format!(
                            "Elements of type {parent} at {} cannot have the following children: {local}",
                            self.path()
                        ),
                        txt1: false,
                    });
                }
            }
            if let Some((_, parents)) = MUST_BE_IN.iter().find(|(e, _)| *e == local) {
                if !parents.contains(&parent.as_str()) {
                    self.out.push(Finding {
                        message: format!(
                            "Elements of type {local} must be in a {}, not a {parent} at {}/{local}",
                            parents.join("|"),
                            self.path()
                        ),
                        txt1: false,
                    });
                }
            }
            if PARAGRAPH_INVALID.contains(&local.as_str()) && self.stack.iter().any(|e| e == "p") {
                self.out.push(Finding {
                    message: format!("Invalid element name inside a paragraph in the XHTML ('{local}')"),
                    txt1: false,
                });
            }
            if BLOCK.contains(&local.as_str()) {
                for j in 0..self.stack.len() {
                    if self.open[j].blocks.len() < MAX_BLOCKS && TEXT_ELEMENTS.contains(&self.stack[j].as_str()) {
                        let rel = format!("{}/{local}", self.stack[j..].join("/"));
                        self.open[j].blocks.push(rel);
                    }
                }
            }
            if local == "a" {
                if let Some(j) = self.stack.iter().position(|e| e == "a") {
                    self.out.push(Finding {
                        message: format!(
                            "Elements of type a at {} cannot contain nested a at {}/a",
                            self.stack[..=j].join("/"),
                            self.stack[j..].join("/")
                        ),
                        txt1: false,
                    });
                }
            }
        }
        for (k, v) in &attrs {
            if k == "xmlns" || k.starts_with("xmlns:") {
                continue;
            }
            if !attribute_allowed(&local, k) {
                self.out.push(Finding {
                    message: format!("Invalid attribute name in the XHTML ('{k}' on '{local}')"),
                    txt1: true,
                });
            }
            if k == "id" {
                self.ids.insert(v.clone());
            }
            if k == "name" && local == "a" {
                self.names.insert(v.clone());
            }
        }
        self.stack.push(local.clone());
        let mut open = Open { qname: qname.clone(), mark, ..Open::default() };
        if local == "a" {
            if let Some((_, href)) = attrs.iter().find(|(k, _)| k == "href") {
                open.link = Some((href.clone(), self.path()));
                self.links_open += 1;
            }
        }
        self.open.push(open);
        if local == "img" {
            self.text = true; // an image is content of its own
            if let Some((_, alt)) = attrs.iter().find(|(k, _)| k == "alt") {
                self.link_text(alt);
            }
            if let Some((_, src)) = attrs.iter().find(|(k, _)| k == "src") {
                // an image's finding hangs off the resource's path, at any depth (probed)
                let at = format!("{}/img", self.label);
                self.check_url(src, &at, "", true);
            }
        }
        if self_closing {
            self.close();
        }
        if truncated {
            self.i = self.s.len(); // the document ends here; what is open is taken as closed
        }
        true
    }

    /// Text under the innermost open hyperlink, for its messages.
    fn link_text(&mut self, text: &str) {
        if self.links_open == 0 {
            return;
        }
        if let Some(o) = self.open.iter_mut().rev().find(|o| o.link.is_some()) {
            o.text.push_str(text);
        }
    }

    /// Close the innermost element: a hyperlink is judged with its text;
    /// the block structures found inside a text element are reported
    /// against it, in document order.
    fn close(&mut self) {
        let open = self.open.pop().unwrap_or_default();
        if let Some((href, link_at)) = open.link {
            self.links_open -= 1;
            let label = open.text.trim().to_owned();
            self.check_url(&href, &link_at, &label, false);
        }
        if !open.blocks.is_empty() {
            let name = self.stack.last().cloned().unwrap_or_default();
            let at = self.path();
            self.out.push(Finding {
                message: format!(
                    "Found block structures inside a paragraph/text element {name} at {at}: {}",
                    open.blocks.join(",")
                ),
                txt1: false,
            });
        }
        self.stack.pop();
        self.ns_decls.truncate(open.mark);
    }

    /// A hyperlink's or an image's URL, as the reference validator checks
    /// it: a `data:` URL only needs its data; otherwise no invalid
    /// character (each occurrence counted, each character named once), a
    /// `#fragment` must resolve (at the end), and — only when the
    /// characters pass — a hyperlink's scheme, exactly as written, is
    /// neither active content nor one that cannot be followed.
    fn check_url(&mut self, url: &str, at: &str, label: &str, image: bool) {
        if url.is_empty() {
            self.out
                .push(Finding { message: "The URL is not valid because '(URL is empty)': ".to_owned(), txt1: false });
            if !image {
                self.out.push(Finding { message: format!("Hyperlink at '{at}' for '{label}' is empty"), txt1: false });
            }
            return;
        }
        if let Some(data) = url.strip_prefix("data:") {
            if !data.contains(',') {
                self.out.push(Finding {
                    message: format!("The URL is not valid because '(No data found in data: URL)': {url}"),
                    txt1: false,
                });
            }
            return; // self-contained: nothing leaves the resource to fetch
        }
        // the host's option, never the oracle's rule: anything but a
        // `#fragment` resolved inside the resource leaves it
        if self.refuse_external && !url.starts_with('#') {
            let what = if image { "image source" } else { "hyperlink" };
            self.out.push(Finding {
                message: format!("The narrative {what} '{url}' at '{at}' is outside the resource"),
                txt1: false,
            });
        }
        let mut count = 0usize;
        let mut bad: Vec<char> = Vec::new();
        for c in url.chars() {
            if URL_INVALID.contains(&c) {
                count += 1;
                if !bad.contains(&c) {
                    bad.push(c);
                }
            }
        }
        if count > 0 {
            bad.sort_unstable();
            let shown: Vec<String> = bad.iter().map(|c| format!("'{c}'")).collect();
            let reason = if count == 1 {
                format!("URL contains Invalid Character: {}", shown[0])
            } else {
                format!("URL contains {count} Invalid Characters: {}", shown.join(","))
            };
            self.out
                .push(Finding { message: format!("The URL is not valid because '({reason})': {url}"), txt1: false });
        }
        if let Some(frag) = url.strip_prefix('#') {
            // a bare `#` is the document itself, which a hyperlink may name
            // and an image may not: `<a href="#">` passes and
            // `<img src="#">` does not resolve (probed, both)
            if !frag.is_empty() || image {
                self.frags.push((frag.to_owned(), at.to_owned(), label.to_owned(), image));
            }
        } else if !image && count == 0 {
            if let Some((scheme, _)) = url.split_once(':') {
                match scheme {
                    "javascript" | "vbscript" => self.out.push(Finding {
                        message: format!(
                            "Hyperlink scheme '{scheme}' in '{url}' at '{at}' for '{label}' has active content, which is a security risk and not allowed"
                        ),
                        txt1: false,
                    }),
                    "urn" | "cid" => self.out.push(Finding {
                        message: format!(
                            "Hyperlink scheme '{scheme}' in '{url}' at '{at}' for '{label}' is not a valid hyperlinkable scheme"
                        ),
                        txt1: false,
                    }),
                    _ => {}
                }
            }
        }
    }

    /// Character references in text or an attribute value, as the
    /// reference parser reads them: XML's five decode; a name it knows from
    /// HTML is an invalid entity (kept as written); any other name, or a
    /// reference without its `;`, is a parse failure; a numeric reference
    /// decodes when it parses as a Java `int` in range, fails when it is
    /// out of range, and passes silently when it does not parse at all.
    /// Returns the decoded value, or None on a parse failure.
    fn decode(&mut self, raw: &str) -> Option<String> {
        let mut out = String::new();
        let mut rest = raw;
        while let Some(k) = rest.find('&') {
            out.push_str(&rest[..k]);
            let after = &rest[k + 1..];
            let end = after.find([';', '<']).unwrap_or(after.len());
            let reference = &after[..end];
            if after.as_bytes().get(end) != Some(&b';') {
                self.fail(format!("Error parsing XHTML: unable to parse character reference '{reference}''"));
                return None;
            }
            if reference.is_empty() {
                self.fail(format!("Error parsing XHTML: Invalid literal declaration following text: {}", &rest[..k]));
                return None;
            }
            if let Some(num) = reference.strip_prefix('#') {
                if num.is_empty() {
                    self.fail("Error parsing XHTML: String index out of range: 1".to_owned());
                    return None;
                }
                let parsed = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => java_int(hex, 16),
                    None => java_int(num, 10),
                };
                match parsed {
                    Some(v) if !(0..=0x10FFFF).contains(&v) => {
                        self.fail(format!("Error parsing XHTML: Not a valid Unicode code point: 0x{:X}", v as u32));
                        return None;
                    }
                    // a lone surrogate passes there (probed) and reads as content here
                    Some(v) => out.push(char::from_u32(v as u32).unwrap_or('\u{fffd}')),
                    None => {
                        out.push('&');
                        out.push_str(reference);
                        out.push(';');
                    }
                }
            } else {
                match reference {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    _ if HTML_ENTITIES.binary_search(&reference).is_ok() => {
                        self.out.push(Finding {
                            message: format!("Invalid entity in the XHTML ('&{reference};')"),
                            txt1: false,
                        });
                        out.push('&');
                        out.push_str(reference);
                        out.push(';');
                    }
                    _ => {
                        self.fail(format!("Error parsing XHTML: unable to parse character reference '{reference}''"));
                        return None;
                    }
                }
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Some(out)
    }
}

/// A name byte: anything but the delimiters (the reference parser takes
/// a digit first and a non-ASCII name, and then judges the name).
fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b':' | b'-' | b'_' | b'.') || c >= 0x80
}

/// `Integer.parseInt(s, radix)`: an optional sign, then digits, within
/// `i32`; None where Java throws. ASCII digits only — Java also reads
/// other scripts' digits, which then decode to a character; here they
/// stay as written, and both count as content (probed: the verdict is
/// the same).
fn java_int(s: &str, radix: u32) -> Option<i32> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if digits.is_empty() {
        return None;
    }
    let mut v: i64 = 0;
    for c in digits.chars() {
        v = v * i64::from(radix) + i64::from(c.to_digit(radix)?);
        if v > i64::from(i32::MAX) + 1 {
            return None;
        }
    }
    let v = if neg { -v } else { v };
    i32::try_from(v).ok()
}

/// The attributes the reference validator allows on an element (probed).
fn attribute_allowed(element: &str, attr: &str) -> bool {
    // on every element (probed on p, div, span, li, ol, tr, td, table, a, img, area, map)
    const GLOBAL: &[&str] = &[
        "abbr",
        "accesskey",
        "align",
        "axis",
        "char",
        "charoff",
        "class",
        "colspan",
        "dir",
        "headers",
        "id",
        "lang",
        "rowspan",
        "scope",
        "span",
        "style",
        "tabindex",
        "title",
        "valign",
        "width",
        "xml:lang",
    ];
    if GLOBAL.contains(&attr) {
        return true;
    }
    let per: &[&str] = match element {
        "a" => &["href", "name", "charset", "coords", "hreflang", "rel", "shape", "type"],
        "img" => &["src", "alt", "border", "height", "ismap", "longdesc", "usemap"],
        "table" => &["border", "cellpadding", "cellspacing", "frame", "rules", "summary"],
        "td" | "th" => &["nowrap"],
        "area" => &["href", "alt", "shape", "coords", "nohref"],
        "map" => &["name"],
        _ => &[],
    };
    per.contains(&attr)
}
