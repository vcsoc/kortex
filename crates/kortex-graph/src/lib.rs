//! Bounded deterministic graph algorithms with evidence paths.
#[cfg(test)]
mod control_tests;
use kortex_core::{Edge, Error, Result, Snapshot};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Upstream,
    Downstream,
    Both,
}
impl Direction {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "upstream" => Ok(Self::Upstream),
            "downstream" => Ok(Self::Downstream),
            "both" => Ok(Self::Both),
            _ => Err(Error::new(
                "INVALID_DIRECTION",
                "expected upstream, downstream or both",
                2,
            )),
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Reach {
    pub symbol_id: String,
    pub depth: usize,
    pub confidence: f64,
    pub edge_path: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Traversal {
    pub items: Vec<Reach>,
    pub edges: Vec<Edge>,
    pub truncated: bool,
    pub ambiguous_relationships: usize,
    pub unresolved_relationships: usize,
}
pub struct Limits {
    pub depth: usize,
    pub max_nodes: usize,
    pub timeout_ms: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            depth: 3,
            max_nodes: 500,
            timeout_ms: 10000,
        }
    }
}
fn dependency(e: &Edge) -> bool {
    matches!(
        e.kind.as_str(),
        "CALLS"
            | "IMPORTS"
            | "REFERENCES"
            | "DEPENDS_ON"
            | "INHERITS"
            | "IMPLEMENTS"
            | "USES_TYPE"
            | "TESTS"
    )
}
pub fn traverse(
    snapshot: &Snapshot,
    start: &str,
    direction: Direction,
    limits: &Limits,
    all_edges: bool,
) -> Result<Traversal> {
    traverse_selected(snapshot, start, direction, limits, all_edges, None)
}
/// Restrict traversal to explicitly evidenced relationship kinds (e.g. process CALLS).
pub fn traverse_kinds(
    snapshot: &Snapshot,
    start: &str,
    direction: Direction,
    limits: &Limits,
    edge_kinds: &[&str],
) -> Result<Traversal> {
    traverse_selected(snapshot, start, direction, limits, false, Some(edge_kinds))
}
fn traverse_selected(
    snapshot: &Snapshot,
    start: &str,
    direction: Direction,
    limits: &Limits,
    all_edges: bool,
    edge_kinds: Option<&[&str]>,
) -> Result<Traversal> {
    if limits.depth > 10 || limits.max_nodes == 0 || limits.max_nodes > 10000 {
        return Err(Error::new(
            "QUERY_LIMIT",
            "depth <=10 and nodes 1..10000 required",
            2,
        ));
    }
    let deadline = Instant::now() + Duration::from_millis(limits.timeout_ms);
    let mut adjacency: BTreeMap<&str, Vec<(&str, &Edge)>> = BTreeMap::new();

    for (index, e) in snapshot.edges.iter().enumerate() {
        if index % 1024 == 0 && Instant::now() >= deadline {
            return Err(Error::new(
                "QUERY_TIMEOUT",
                "graph adjacency exceeded deadline",
                9,
            ));
        }
        if edge_kinds.is_some_and(|kinds| !kinds.contains(&e.kind.as_str()))
            || (!all_edges && !dependency(e))
        {
            continue;
        }
        // Candidate relationships never become executable/dependency paths merely
        // because malformed/legacy input happens to carry a target_id.
        if !matches!(e.resolution.as_str(), "exact" | "inferred") {
            continue;
        }
        let Some(target) = e.target_id.as_deref() else {
            continue;
        };
        if direction != Direction::Upstream {
            adjacency.entry(&e.source_id).or_default().push((target, e));
        }
        if direction != Direction::Downstream {
            adjacency.entry(target).or_default().push((&e.source_id, e));
        }
    }
    let mut queue = VecDeque::from([(start.to_owned(), 0, 1.0_f64, Vec::<String>::new())]);
    let mut seen = BTreeSet::from([start.to_owned()]);
    let mut items = vec![];
    let mut edges = BTreeMap::new();
    let mut truncated = false;
    while let Some((node, depth, confidence, path)) = queue.pop_front() {
        if Instant::now() > deadline {
            return Err(Error::new(
                "QUERY_TIMEOUT",
                "graph query exceeded deadline",
                9,
            ));
        }
        for (index, (next, e)) in adjacency
            .get(node.as_str())
            .into_iter()
            .flatten()
            .enumerate()
        {
            if index % 1024 == 0 && Instant::now() >= deadline {
                return Err(Error::new(
                    "QUERY_TIMEOUT",
                    "graph expansion exceeded deadline",
                    9,
                ));
            }
            if depth >= limits.depth {
                if !seen.contains(*next) {
                    truncated = true;
                }
                continue;
            }
            if !seen.contains(*next) && seen.len() >= limits.max_nodes {
                truncated = true;
                continue;
            }
            edges.insert(e.id.clone(), (*e).clone());
            if seen.insert((*next).to_owned()) {
                let mut edge_path = path.clone();
                edge_path.push(e.id.clone());
                let c = confidence.min(e.confidence);
                items.push(Reach {
                    symbol_id: (*next).into(),
                    depth: depth + 1,
                    confidence: c,
                    edge_path: edge_path.clone(),
                });
                queue.push_back(((*next).into(), depth + 1, c, edge_path));
            }
        }
    }
    items.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.symbol_id.cmp(&b.symbol_id)));
    let mut ambiguous = 0;
    let mut unresolved = 0;
    for (index, e) in snapshot.edges.iter().enumerate() {
        if index % 1024 == 0 && Instant::now() >= deadline {
            return Err(Error::new(
                "QUERY_TIMEOUT",
                "graph ambiguity scan exceeded deadline",
                9,
            ));
        }
        if edge_kinds.is_some_and(|kinds| !kinds.contains(&e.kind.as_str()))
            || (!all_edges && !dependency(e))
        {
            continue;
        }
        if e.resolution == "unresolved" && seen.contains(&e.source_id) {
            unresolved += 1;
        }
        if e.resolution == "ambiguous"
            && (seen.contains(&e.source_id) || e.candidates.iter().any(|c| seen.contains(c)))
        {
            ambiguous += 1;
        }
    }
    Ok(Traversal {
        items,
        edges: edges.into_values().collect(),
        truncated,
        ambiguous_relationships: ambiguous,
        unresolved_relationships: unresolved,
    })
}
pub fn shortest_path(
    snapshot: &Snapshot,
    from: &str,
    to: &str,
    limits: &Limits,
) -> Result<Option<Vec<Edge>>> {
    if from == to {
        return Ok(Some(vec![]));
    }
    let t = traverse(snapshot, from, Direction::Downstream, limits, false)?;
    Ok(t.items.iter().find(|n| n.symbol_id == to).map(|n| {
        n.edge_path
            .iter()
            .filter_map(|id| snapshot.edges.iter().find(|e| e.id == *id).cloned())
            .collect()
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn e(a: &str, b: &str) -> Edge {
        Edge {
            id: format!("{a}-{b}"),
            repo_id: "r".into(),
            source_id: a.into(),
            target_id: Some(b.into()),
            kind: "CALLS".into(),
            source_span: Default::default(),
            resolution: "exact".into(),
            confidence: 1.,
            resolver: "test".into(),
            evidence: "test".into(),
            candidates: vec![],
        }
    }
    #[test]
    fn exhaustive_four_node_reachability_and_evidence_paths() {
        let pairs: Vec<_> = (0..4)
            .flat_map(|a| (0..4).filter(move |b| *b != a).map(move |b| (a, b)))
            .collect();
        for mask in 0u32..(1 << pairs.len()) {
            let mut matrix = [[false; 4]; 4];
            let mut edges = vec![];
            for (bit, (a, b)) in pairs.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    matrix[*a][*b] = true;
                    edges.push(e(&a.to_string(), &b.to_string()));
                }
            }
            for k in 0..4 {
                for a in 0..4 {
                    for b in 0..4 {
                        matrix[a][b] |= matrix[a][k] && matrix[k][b];
                    }
                }
            }
            let snapshot = Snapshot {
                edges,
                ..Snapshot::default()
            };
            for (start, row) in matrix.iter().enumerate() {
                let t = traverse(
                    &snapshot,
                    &start.to_string(),
                    Direction::Downstream,
                    &Limits::default(),
                    false,
                )
                .unwrap();
                let expected: BTreeSet<_> = row
                    .iter()
                    .enumerate()
                    .filter(|(i, reachable)| **reachable && *i != start)
                    .map(|(i, _)| i.to_string())
                    .collect();
                assert_eq!(
                    t.items
                        .iter()
                        .map(|r| r.symbol_id.clone())
                        .collect::<BTreeSet<_>>(),
                    expected
                );
                for reach in &t.items {
                    let mut at = start.to_string();
                    for id in &reach.edge_path {
                        let edge = snapshot.edges.iter().find(|e| e.id == *id).unwrap();
                        assert_eq!(edge.source_id, at);
                        at = edge.target_id.clone().unwrap();
                    }
                    assert_eq!(at, reach.symbol_id);
                    assert_eq!(reach.depth, reach.edge_path.len());
                }
            }
        }
    }
    #[test]
    fn cycles_directions_limits_and_paths() {
        let s = Snapshot {
            edges: vec![e("a", "b"), e("b", "c"), e("c", "a")],
            ..Snapshot::default()
        };
        let l = Limits::default();
        let t = traverse(&s, "c", Direction::Upstream, &l, false).unwrap();
        assert_eq!(t.items[0].symbol_id, "b");
        assert_eq!(t.items[1].symbol_id, "a");
        assert_eq!(shortest_path(&s, "a", "c", &l).unwrap().unwrap().len(), 2);
        assert!(shortest_path(&s, "a", "missing", &l).unwrap().is_none());
        assert!(
            traverse(
                &s,
                "a",
                Direction::Downstream,
                &Limits { max_nodes: 1, ..l },
                false
            )
            .unwrap()
            .truncated
        );
    }
}
