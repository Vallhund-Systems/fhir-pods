//! An owned JSON tree — what lowering builds and de-identification rewrites —
//! serialised back to text with the same escaping the wire uses.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::json::Value;
use crate::wire::escape;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
    Array(Vec<Node>),
    Object(Vec<(String, Node)>),
}

impl Node {
    pub fn from_value(v: &Value<'_>) -> Node {
        match v {
            Value::Null => Node::Null,
            Value::Bool(b) => Node::Bool(*b),
            Value::Number(n) => Node::Number((*n).into()),
            Value::Str(s) => Node::Str(s.to_string()),
            Value::Array(xs) => Node::Array(xs.iter().map(Node::from_value).collect()),
            Value::Object(kvs) => Node::Object(kvs.iter().map(|(k, v)| (k.to_string(), Node::from_value(v))).collect()),
        }
    }

    /// The tree as JSON text.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    pub fn write(&self, out: &mut String) {
        match self {
            Node::Null => out.push_str("null"),
            Node::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Node::Number(n) => out.push_str(n),
            Node::Str(s) => {
                out.push('"');
                escape(out, s);
                out.push('"');
            }
            Node::Array(xs) => {
                out.push('[');
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    x.write(out);
                }
                out.push(']');
            }
            Node::Object(kvs) => {
                out.push('{');
                for (i, (k, v)) in kvs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    escape(out, k);
                    out.push_str("\":");
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

    pub fn member(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Object(kvs) => kvs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn member_mut(&mut self, key: &str) -> Option<&mut Node> {
        match self {
            Node::Object(kvs) => kvs.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The member at `key`, inserted as `make` when absent. A scalar where
    /// an object was expected is replaced (the caller reports the collision).
    pub fn member_or_insert(&mut self, key: &str, make: Node) -> &mut Node {
        if let Node::Object(kvs) = self {
            if let Some(i) = kvs.iter().position(|(k, _)| k == key) {
                return &mut kvs[i].1;
            }
            kvs.push((key.into(), make));
            return &mut kvs.last_mut().unwrap().1;
        }
        *self = Node::Object(alloc::vec![(key.into(), make)]);
        self.member_mut(key).unwrap()
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            _ => None,
        }
    }
}
