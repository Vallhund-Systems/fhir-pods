use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use fhir_pods_core::{json, node::Node};

pub fn call(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fhir-pods"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

pub fn parse(input: &[u8]) -> Node {
    Node::from_value(&json::parse(input).unwrap())
}

pub fn result(output: &Output, code: i32) -> Node {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if code == 2 {
        assert!(output.stdout.is_empty());
        let error = parse(&output.stderr);
        assert!(error.member("error").and_then(Node::as_str).is_some());
        error
    } else {
        assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
        parse(&output.stdout)
    }
}

pub fn array(value: &Node) -> &[Node] {
    let Node::Array(items) = value else { panic!("expected an array: {value:?}") };
    items
}

pub struct Files(pub PathBuf);
impl Files {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "fhir-pods-cli-extra-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub fn write(&self, name: &str, data: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, data).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
