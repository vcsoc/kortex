use super::*;
fn edge(id: &str, source: &str, target: &str, kind: &str) -> Edge {
    Edge {
        id: id.into(),
        repo_id: "test".into(),
        source_id: source.into(),
        target_id: Some(target.into()),
        kind: kind.into(),
        source_span: Default::default(),
        resolution: "exact".into(),
        confidence: 1.0,
        resolver: "fixture".into(),
        evidence: "fixture-specific relationship".into(),
        candidates: vec![],
    }
}
#[test]
fn process_call_filter_does_not_relabel_types_or_ambiguous_candidates_as_calls() {
    let mut ambiguous = edge("ambiguous", "entry", "candidate", "CALLS");
    ambiguous.resolution = "ambiguous".into();
    ambiguous.candidates = vec!["candidate".into()];
    let snapshot = Snapshot {
        edges: vec![
            edge("call", "entry", "worker", "CALLS"),
            edge("type", "worker", "Type", "USES_TYPE"),
            ambiguous,
        ],
        ..Default::default()
    };
    let traversal = traverse_kinds(
        &snapshot,
        "entry",
        Direction::Downstream,
        &Limits::default(),
        &["CALLS"],
    )
    .unwrap();
    assert_eq!(traversal.items.len(), 1);
    assert_eq!(traversal.items[0].symbol_id, "worker");
    assert_eq!(traversal.ambiguous_relationships, 1);
    assert!(
        traverse(
            &snapshot,
            "entry",
            Direction::Downstream,
            &Limits::default(),
            false
        )
        .unwrap()
        .items
        .iter()
        .any(|r| r.symbol_id == "Type")
    );
}
#[test]
fn zero_deadline_aborts_adjacency_build_and_high_degree_queries() {
    let snapshot = Snapshot {
        edges: (0..3000)
            .map(|i| edge(&i.to_string(), "entry", &format!("n{i}"), "CALLS"))
            .collect(),
        ..Default::default()
    };
    let error = traverse(
        &snapshot,
        "entry",
        Direction::Downstream,
        &Limits {
            timeout_ms: 0,
            ..Default::default()
        },
        false,
    )
    .unwrap_err();
    assert_eq!(error.exit, 9);
}

#[test]
fn graph_policies_do_not_extend_parent_deadline_or_detach_shared_cancellation() {
    let snapshot = Snapshot {
        edges: (0..100_000)
            .map(|n| edge(&n.to_string(), "entry", &format!("n{n}"), "CALLS"))
            .collect(),
        ..Default::default()
    };
    let parent = RequestControl::new(Duration::from_secs(30)).unwrap();
    let child = parent.restrict_deadline(parent.deadline() + Duration::from_secs(30));
    parent.cancel();
    let error = traverse_with_control(
        &snapshot,
        "entry",
        Direction::Both,
        &Limits::default(),
        false,
        &child,
    )
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    let short = RequestControl::from_deadline(Instant::now() + Duration::from_millis(1));
    let error = traverse_kinds_with_control(
        &snapshot,
        "entry",
        Direction::Downstream,
        &Limits::default(),
        &["CALLS"],
        &short,
    )
    .unwrap_err();
    assert_eq!(error.code, "TIMEOUT");
}
