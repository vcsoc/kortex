use kortex_core::{Edge, Error, Result, Snapshot, Span};
use std::io::{self, Write};

#[derive(Debug, Clone)]
pub struct ResolutionLimits {
    pub max_edges: usize,
    pub max_candidate_memberships: usize,
    pub max_edge_bytes: usize,
    pub max_imports: usize,
    pub max_fact_bytes: usize,
}
pub(super) struct Budget<'a> {
    pub(super) limits: Option<&'a ResolutionLimits>,
    edges: usize,
    memberships: usize,
    bytes: usize,
}
pub(super) fn reserve(name: &str, used: usize, n: usize, cap: usize) -> Result<usize> {
    match used.checked_add(n) {
        Some(total) if total <= cap => Ok(total),
        total => Err(Error::new(
            "RESOURCE_LIMIT",
            format!(
                "{name}: limit {cap}, attempted {}",
                total.map_or_else(|| "overflow".into(), |n| n.to_string())
            ),
            9,
        )),
    }
}
#[derive(Default)]
pub(super) struct Counter(pub usize);
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("JSON size overflow"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl<'a> Budget<'a> {
    pub(super) fn new(limits: Option<&'a ResolutionLimits>) -> Self {
        Self {
            limits,
            edges: 0,
            memberships: 0,
            bytes: 0,
        }
    }
    pub(super) fn memberships(&mut self, n: usize) -> Result<()> {
        if let Some(limits) = self.limits {
            self.memberships = reserve(
                "candidate memberships",
                self.memberships,
                n,
                limits.max_candidate_memberships,
            )?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn edge(
        &mut self,
        snapshot: &Snapshot,
        source: &str,
        target: Option<&str>,
        kind: &str,
        span: Span,
        resolution: &str,
        candidates: &[&str],
        evidence: &str,
    ) -> Result<Edge> {
        if let Some(limits) = self.limits {
            self.edges = reserve("edges", self.edges, 1, limits.max_edges)?;
        }
        let mut edge = super::edge(
            snapshot,
            source,
            target,
            kind,
            span,
            resolution,
            vec![],
            evidence,
        );
        if let Some(limits) = self.limits {
            let mut size = Counter::default();
            serde_json::to_writer(&mut size, &edge)?;
            let mut candidate_size = Counter::default();
            serde_json::to_writer(&mut candidate_size, candidates)?;
            // Replace the empty [] in the base edge without allocating candidate strings.
            let bytes = reserve(
                "edge JSON bytes",
                size.0 - 2,
                candidate_size.0,
                limits.max_edge_bytes,
            )?;
            self.bytes = reserve("edge JSON bytes", self.bytes, bytes, limits.max_edge_bytes)?;
        }
        edge.candidates = candidates.iter().map(|id| (*id).to_owned()).collect();
        Ok(edge)
    }
}
