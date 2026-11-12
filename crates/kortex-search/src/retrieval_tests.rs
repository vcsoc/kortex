use super::*;
use crate::{
    Filters,
    retrieval::{HybridOptions, query_hybrid},
};
#[test]
fn actual_index_native_encoder_vector_rrf_and_cooperative_cancellation() {
    let (_artifacts, manifest) = fixture();
    let mut model = NativeBert::load(manifest).unwrap();
    let mut cache = EmbeddingCache::new(100).unwrap();
    let repository = tempfile::tempdir().unwrap();
    std::fs::write(
        repository.path().join("code.py"),
        "def hello():\n    pass\ndef search():\n    hello()\n",
    )
    .unwrap();
    let report = kortex_indexer::index(
        repository.path(),
        &kortex_config::Config::default(),
        &Default::default(),
    )
    .unwrap();
    let store = kortex_storage::Store::open(repository.path(), false).unwrap();
    let result = query_hybrid(
        &store,
        &report.snapshot,
        "hello",
        &mut model,
        &mut cache,
        &HybridOptions {
            filters: Filters {
                kind: Some("function"),
                ..Default::default()
            },
            max_candidates: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.algorithm, "weighted_rrf_v1");
    assert_eq!(result.candidates_examined, 1);
    assert!(result.partial);
    assert!(
        result
            .items
            .iter()
            .any(|h| h.why_matched.iter().any(|why| why == "local_model_vector"))
    );
    assert_eq!(result.model_fingerprint, model.model().fingerprint());
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    let error = query_hybrid(
        &store,
        &report.snapshot,
        "hello",
        &mut model,
        &mut cache,
        &HybridOptions {
            cancelled: Some(&cancelled),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "QUERY_CANCELLED");
    let error = query_hybrid(
        &store,
        &report.snapshot,
        "hello",
        &mut model,
        &mut cache,
        &HybridOptions {
            deadline: std::time::Instant::now(),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.exit, 9);
    let control = kortex_core::RequestControl::new(std::time::Duration::from_secs(30)).unwrap();
    let mut cancelling = CancelAfterNative {
        inner: model,
        control: control.clone(),
        calls: 0,
    };
    let mut fresh_cache = EmbeddingCache::new(100).unwrap();
    let error = crate::retrieval::query_hybrid_with_control(
        &store,
        &report.snapshot,
        "hello",
        &mut cancelling,
        &mut fresh_cache,
        &HybridOptions::default(),
        &control,
    )
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    assert_eq!(
        cancelling.calls, 1,
        "actual native forward completes, then shared cancellation must prevent document embedding/ranking"
    );
}
struct CancelAfterNative {
    inner: NativeBert,
    control: kortex_core::RequestControl,
    calls: usize,
}
impl LocalEmbedder for CancelAfterNative {
    fn model(&self) -> &VerifiedModel {
        self.inner.model()
    }
    fn cache_namespace(&self) -> &str {
        self.inner.cache_namespace()
    }
    fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        let vector = self.inner.embed(text)?;
        self.calls += 1;
        self.control.cancel();
        Ok(vector)
    }
}
struct Counting {
    inner: NativeBert,
    calls: usize,
}
impl LocalEmbedder for Counting {
    fn model(&self) -> &VerifiedModel {
        self.inner.model()
    }
    fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        self.calls += 1;
        self.inner.embed(text)
    }
}
#[test]
fn real_native_cache_eviction_honors_byte_budget_independently_of_entry_limit() {
    let (_artifacts, manifest) = fixture();
    let mut provider = Counting {
        inner: NativeBert::load(manifest).unwrap(),
        calls: 0,
    };
    let mut cache = EmbeddingCache::with_memory_limit(100, 32).unwrap();
    for text in ["hello", "world", "code", "search"] {
        cache.get_or_embed(&mut provider, text).unwrap();
    }
    assert_eq!(provider.calls, 4);
    cache.get_or_embed(&mut provider, "hello").unwrap();
    assert_eq!(
        provider.calls, 5,
        "the byte limit evicted an actual native vector before the entry limit"
    );
    assert!(EmbeddingCache::with_memory_limit(1, 64 * 1024 * 1024 + 1).is_err());
}
#[test]
fn cache_key_includes_actual_content_and_verified_model_revision_and_evicts() {
    let (_artifacts, manifest) = fixture();
    let mut provider = Counting {
        inner: NativeBert::load(manifest.clone()).unwrap(),
        calls: 0,
    };
    let mut cache = EmbeddingCache::new(1).unwrap();
    cache.get_or_embed(&mut provider, "hello").unwrap();
    cache.get_or_embed(&mut provider, "hello").unwrap();
    assert_eq!(provider.calls, 1);
    cache.get_or_embed(&mut provider, "world").unwrap();
    assert_eq!(provider.calls, 2);
    cache.get_or_embed(&mut provider, "hello").unwrap();
    assert_eq!(provider.calls, 3, "bounded FIFO eviction");
    let mut changed = manifest;
    changed.revision = "fixture-v2".into();
    let mut revised = Counting {
        inner: NativeBert::load(changed).unwrap(),
        calls: 0,
    };
    cache.get_or_embed(&mut revised, "hello").unwrap();
    assert_eq!(
        revised.calls, 1,
        "same content and weights but changed revision must not reuse old cache"
    );
}
