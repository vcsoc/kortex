use kortex_core::{Error, ParsedFile, RequestControl, Result};
use std::io::{self, Write};
use tree_sitter::Node;

#[derive(Debug, Clone)]
pub struct ParseLimits {
    pub max_ast_nodes: usize,
    pub max_ast_depth: usize,
}

#[cfg(test)]
pub(super) fn admit(root: Node<'_>, limits: &ParseLimits) -> Result<()> {
    admit_with_control(root, limits, None)
}
pub(super) fn admit_with_control(
    root: Node<'_>,
    limits: &ParseLimits,
    control: Option<&RequestControl>,
) -> Result<()> {
    let mut cursor = root.walk();
    let mut nodes = 0usize;
    let mut depth = 0usize;
    loop {
        if let Some(control) = control {
            control.check()?;
        }
        nodes += 1;
        for (name, n, cap) in [
            ("file AST nodes", nodes, limits.max_ast_nodes),
            ("file AST depth", depth, limits.max_ast_depth.min(192)),
        ] {
            if n > cap {
                return Err(Error::new(
                    "RESOURCE_LIMIT",
                    format!("{name}: limit {cap}, attempted {n}"),
                    9,
                ));
            }
        }
        if cursor.goto_first_child() {
            depth += 1;
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return Ok(());
            }
            depth -= 1;
        }
    }
}

struct Counter {
    bytes: usize,
    cap: usize,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self.bytes.checked_add(bytes.len()) {
            Some(n) if n <= self.cap => {
                self.bytes = n;
                Ok(bytes.len())
            }
            _ => Err(io::Error::other("parsed JSON byte limit")),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
/// Exact serialized size, without allocating a serialized copy. Refuses at the cap.
pub fn parsed_file_bytes(file: &ParsedFile, cap: usize) -> Result<usize> {
    let mut counter = Counter { bytes: 0, cap };
    serde_json::to_writer(&mut counter, file).map_err(|_| {
        Error::new(
            "RESOURCE_LIMIT",
            format!("parsed JSON bytes: limit {cap} exceeded"),
            9,
        )
    })?;
    Ok(counter.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_tree_node_and_depth_boundaries() {
        let source = "fn leaf() {}";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::super::grammar("rust", "a.rs").unwrap())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let nodes = root.descendant_count();
        assert!(
            admit(
                root,
                &ParseLimits {
                    max_ast_nodes: nodes,
                    max_ast_depth: 192
                }
            )
            .is_ok()
        );
        assert_eq!(
            admit(
                root,
                &ParseLimits {
                    max_ast_nodes: nodes - 1,
                    max_ast_depth: 192
                }
            )
            .unwrap_err()
            .code,
            "RESOURCE_LIMIT"
        );
        assert_eq!(
            admit(
                root,
                &ParseLimits {
                    max_ast_nodes: nodes,
                    max_ast_depth: 1
                }
            )
            .unwrap_err()
            .code,
            "RESOURCE_LIMIT"
        );
    }
}
