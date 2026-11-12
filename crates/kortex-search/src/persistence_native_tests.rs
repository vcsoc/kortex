//! Genuine generated-BERT vectors; no pretrained quality claim or model download.
#![cfg(unix)]
use super::*;
use crate::{
    hybrid::{LocalEmbedder, VectorPersistence, VerifiedModel},
    persistence::SqliteVectorPersistence,
};
use kortex_core::id;

struct CountingNative {
    native: NativeBert,
    calls: usize,
}
impl LocalEmbedder for CountingNative {
    fn model(&self) -> &VerifiedModel {
        self.native.model()
    }
    fn cache_namespace(&self) -> &str {
        self.native.cache_namespace()
    }
    fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        self.calls += 1;
        self.native.embed(text)
    }
}
#[test]
#[cfg(unix)]
fn native_vectors_survive_restart_and_model_content_keys_invalidate_exactly() {
    let (_artifacts, manifest) = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("vectors");
    let mut provider = CountingNative {
        native: NativeBert::load(manifest.clone()).unwrap(),
        calls: 0,
    };
    let mut cache = EmbeddingCache::with_persistence(
        10,
        Box::new(SqliteVectorPersistence::open(&path, 10, 4096).unwrap()),
    )
    .unwrap();
    let first = cache.get_or_embed(&mut provider, "hello world").unwrap();
    assert_eq!(provider.calls, 1);
    drop(cache);
    let mut cache = EmbeddingCache::with_persistence(
        10,
        Box::new(SqliteVectorPersistence::open(&path, 10, 4096).unwrap()),
    )
    .unwrap();
    assert_eq!(
        cache.get_or_embed(&mut provider, "hello world").unwrap(),
        first
    );
    assert_eq!(
        provider.calls, 1,
        "restart must retrieve real persisted vector without another forward pass"
    );
    assert_ne!(
        cache.get_or_embed(&mut provider, "code search").unwrap(),
        first
    );
    assert_eq!(provider.calls, 2);
    let mut revised = manifest;
    revised.revision = "actual-model-revision-2".into();
    let mut changed = CountingNative {
        native: NativeBert::load(revised).unwrap(),
        calls: 0,
    };
    assert_eq!(
        cache.get_or_embed(&mut changed, "hello world").unwrap(),
        first
    );
    assert_eq!(
        changed.calls, 1,
        "model version changes must miss even when artifact bytes agree"
    );
    drop(cache);
    for name in ["vectors-v1.db", "vectors-v1.db-journal"] {
        let bytes = std::fs::read(path.join(name)).unwrap();
        for source in [
            b"hello world".as_slice(),
            b"code search".as_slice(),
            b"actual-model-revision-2".as_slice(),
        ] {
            assert!(
                !bytes.windows(source.len()).any(|s| s == source),
                "cache must not contain source or model labels"
            );
        }
    }
}
#[test]
#[cfg(unix)]
fn concurrent_native_vector_writes_restart_fifo_and_byte_eviction_are_real() {
    let (_artifacts, manifest) = fixture();
    let mut provider = NativeBert::load(manifest).unwrap();
    let vectors = ["hello", "world", "code search"].map(|text| provider.embed(text).unwrap());
    let model = id(&[provider.cache_namespace(), provider.model().fingerprint()]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("concurrent");
    std::thread::scope(|scope| {
        for worker in 0..4 {
            let path = &path;
            let model = &model;
            let vectors = &vectors;
            scope.spawn(move || {
                let mut cache = SqliteVectorPersistence::open(path, 32, 8192).unwrap();
                for n in 0..8 {
                    let key = hash(format!("thread-{worker}-{n}"));
                    cache.put(model, &key, &vectors[n % 3]).unwrap();
                    assert_eq!(cache.get(model, &key, 4).unwrap().unwrap(), vectors[n % 3]);
                }
            });
        }
    });
    let mut cache = SqliteVectorPersistence::open(&path, 32, 8192).unwrap();
    assert_eq!(cache.usage().unwrap(), (32, 32 * 232));
    for worker in 0..4 {
        for n in 0..8 {
            assert_eq!(
                cache
                    .get(&model, &hash(format!("thread-{worker}-{n}")), 4)
                    .unwrap()
                    .unwrap(),
                vectors[n % 3]
            );
        }
    }
    drop(cache);
    let mut cache = SqliteVectorPersistence::open(&path, 2, 8192).unwrap();
    assert_eq!(cache.usage().unwrap().0, 2);
    for (n, vector) in vectors.iter().enumerate() {
        cache
            .put(&model, &hash(format!("new-{n}")), vector)
            .unwrap();
    }
    assert!(cache.get(&model, &hash("new-0"), 4).unwrap().is_none());
    assert_eq!(
        cache.get(&model, &hash("new-2"), 4).unwrap().unwrap(),
        vectors[2]
    );
    drop(cache);
    let mut cache = SqliteVectorPersistence::open(&path, 100, 256).unwrap();
    assert_eq!(cache.usage().unwrap(), (1, 232));
    cache.put(&model, &hash("final"), &vectors[0]).unwrap();
    assert_eq!(cache.usage().unwrap(), (1, 232));
    assert_eq!(
        cache.get(&model, &hash("final"), 4).unwrap().unwrap(),
        vectors[0]
    );
    drop(cache);
    let cache = SqliteVectorPersistence::open(&path, 100, 8192).unwrap();
    assert_eq!(
        cache.usage().unwrap(),
        (1, 232),
        "reopen cannot silently increase persisted bounds"
    );
}

#[test]
#[cfg(unix)]
fn actual_native_cache_propagates_sql_failure_and_refuses_corruption_after_restart() {
    let (_artifacts, manifest) = fixture();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("failed-cache");
    let mut provider = CountingNative {
        native: NativeBert::load(manifest).unwrap(),
        calls: 0,
    };
    let mut cache = EmbeddingCache::with_persistence(
        10,
        Box::new(SqliteVectorPersistence::open(&path, 10, 4096).unwrap()),
    )
    .unwrap();
    let first = cache.get_or_embed(&mut provider, "hello world").unwrap();
    let db = rusqlite::Connection::open(path.join("vectors-v1.db")).unwrap();
    db.execute_batch("PRAGMA journal_mode=TRUNCATE; PRAGMA synchronous=FULL; CREATE TRIGGER fail_native BEFORE INSERT ON vectors BEGIN SELECT RAISE(ABORT,'real SQLite failure'); END;").unwrap();
    for expected_calls in [2, 3] {
        assert_eq!(
            cache
                .get_or_embed(&mut provider, "code search")
                .unwrap_err()
                .code,
            "VECTOR_CACHE_STORAGE"
        );
        assert_eq!(
            provider.calls, expected_calls,
            "failed persistence cannot enter the memory cache"
        );
    }
    assert_eq!(
        cache.get_or_embed(&mut provider, "hello world").unwrap(),
        first
    );
    db.execute_batch("DROP TRIGGER fail_native").unwrap();
    cache.get_or_embed(&mut provider, "code search").unwrap();
    drop(cache);
    db.execute("UPDATE vectors SET checksum=?1", ["0".repeat(64)])
        .unwrap();
    drop(db);
    assert_eq!(
        SqliteVectorPersistence::open(&path, 1, 256)
            .err()
            .unwrap()
            .code,
        "VECTOR_CACHE_CORRUPT",
        "shrink eviction must not hide corrupt actual native vectors"
    );
    assert!(std::fs::metadata(path.join("vectors-v1.db")).unwrap().len() > 0);
}
