//! Declaration-evidenced inheritance/override edges, never name-only workspace guesses.
use super::Lookup;
use kortex_core::Edge;
use std::collections::BTreeSet;
pub(super) fn complete(snapshot: &Lookup<'_>, edges: &mut Vec<Edge>) {
    for e in edges.iter_mut() {
        if e.kind == "INHERITS"
            && e.target_id
                .as_deref()
                .and_then(|id| snapshot.symbol(id))
                .is_some_and(|s| s.kind == "interface")
            && snapshot
                .symbol(&e.source_id)
                .is_some_and(|s| matches!(s.kind.as_str(), "class" | "struct"))
        {
            *e = super::edge(
                snapshot,
                &e.source_id,
                e.target_id.as_deref(),
                "IMPLEMENTS",
                e.source_span.clone(),
                &e.resolution,
                e.candidates.clone(),
                &e.evidence,
            );
        }
    }
    for method in snapshot
        .all_symbols()
        .filter(|s| s.attributes["override"] == true)
    {
        let Some(owner) = method.owner_id.as_deref() else {
            continue;
        };
        let mut bases = BTreeSet::new();
        let mut uncertain = method.attributes["syntax_error"] == true;
        for e in edges.iter().filter(|e| {
            e.source_id == owner && matches!(e.kind.as_str(), "INHERITS" | "IMPLEMENTS")
        }) {
            uncertain |= e.resolution != "exact";
            if let Some(target) = &e.target_id {
                bases.insert(target.as_str());
            }
            bases.extend(e.candidates.iter().map(String::as_str));
        }
        let candidates: BTreeSet<_> = snapshot
            .named(&method.language, &method.name)
            .filter(|s| {
                s.owner_id
                    .as_deref()
                    .is_some_and(|owner| bases.contains(owner))
            })
            .map(|s| s.symbol_id.clone())
            .collect();
        let candidates: Vec<_> = candidates.into_iter().collect();
        let resolution = match candidates.len() {
            0 => "unresolved",
            1 if uncertain => "inferred",
            1 => "exact",
            _ => "ambiguous",
        };
        edges.push(super::edge(
            snapshot,
            &method.symbol_id,
            if candidates.len() == 1 {
                Some(&candidates[0])
            } else {
                None
            },
            "OVERRIDES",
            method.span.clone(),
            resolution,
            if candidates.len() > 1 {
                candidates.clone()
            } else {
                vec![]
            },
            "explicit override marker and declared base-member set",
        ));
    }
}
