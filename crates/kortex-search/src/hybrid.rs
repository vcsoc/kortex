//! Opt-in local embedding integration contracts. No bundled model, downloads or remote provider.
//! This module supplies artifact verification, a bounded content/model cache and RRF;
//! it does NOT pretend keyword hashes are neural model inference.
use kortex_core::{Error, Result, hash, id};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub path: PathBuf,
    pub blake3: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalModelManifest {
    pub schema_version: String,
    pub model_id: String,
    pub revision: String,
    pub license: String,
    pub architecture: String,
    pub model: Artifact,
    pub tokenizer: Artifact,
    pub config: Artifact,
    pub dimensions: usize,
    pub max_tokens: usize,
    pub pooling: String,
}
#[derive(Debug, Clone)]
pub struct VerifiedModel {
    manifest: LocalModelManifest,
    fingerprint: String,
}
impl VerifiedModel {
    pub fn manifest(&self) -> &LocalModelManifest {
        &self.manifest
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}
fn invalid(message: &str) -> Error {
    Error::new("INVALID_LOCAL_MODEL", message, 2)
}
fn verify_artifact(artifact: &Artifact, max_bytes: u64) -> Result<()> {
    if !artifact.path.is_absolute()
        || artifact.blake3.len() != 64
        || !artifact
            .blake3
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(invalid(
            "artifact requires an explicit absolute local path and lowercase BLAKE3 checksum",
        ));
    }
    let mut file = File::open(&artifact.path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > max_bytes {
        return Err(invalid(
            "artifact exceeds size policy or is not a regular file",
        ));
    }
    let mut digest = blake3::Hasher::new();
    let mut buffer = [0; 65536];
    let mut total = 0_u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > max_bytes {
            return Err(invalid("artifact grew beyond policy"));
        }
        digest.update(&buffer[..n]);
    }
    if digest.finalize().to_hex().as_str() != artifact.blake3 {
        return Err(invalid("artifact checksum mismatch"));
    }
    Ok(())
}
/// Verifies bytes, not architecture support or legal approval; inference adapters must
/// revalidate/check the same bytes at load to prevent artifact replacement races.
pub fn verify_model(manifest: LocalModelManifest) -> Result<VerifiedModel> {
    if manifest.schema_version != "1.0"
        || manifest.model_id.is_empty()
        || manifest.model_id.len() > 256
        || manifest.revision.is_empty()
        || manifest.revision.len() > 256
        || manifest.license.is_empty()
        || manifest.architecture.is_empty()
        || !(1..=4096).contains(&manifest.dimensions)
        || !(1..=8192).contains(&manifest.max_tokens)
        || !matches!(manifest.pooling.as_str(), "attention_mean_l2" | "cls_l2")
    {
        return Err(invalid(
            "versioned identity, license, architecture, dimensions, token bound and supported pooling required",
        ));
    }
    verify_artifact(&manifest.model, 512 * 1024 * 1024)?;
    verify_artifact(&manifest.tokenizer, 32 * 1024 * 1024)?;
    verify_artifact(&manifest.config, 1024 * 1024)?;
    let fingerprint = id(&[
        &manifest.schema_version,
        &manifest.model_id,
        &manifest.revision,
        &manifest.architecture,
        &manifest.model.blake3,
        &manifest.tokenizer.blake3,
        &manifest.config.blake3,
        &manifest.dimensions.to_string(),
        &manifest.max_tokens.to_string(),
        &manifest.pooling,
    ]);
    Ok(VerifiedModel {
        manifest,
        fingerprint,
    })
}
/// Explicit library opt-in only. A real native inference adapter must implement this;
/// no provider is auto-created from repository configuration and none is bundled here.
pub trait LocalEmbedder {
    fn model(&self) -> &VerifiedModel;
    fn embed(&mut self, text: &str) -> Result<Vec<f32>>;
}
pub struct EmbeddingCache {
    capacity: usize,
    values: BTreeMap<String, Vec<f32>>,
    order: VecDeque<String>,
}
impl EmbeddingCache {
    pub fn new(capacity: usize) -> Result<Self> {
        if capacity == 0 || capacity > 100_000 {
            return Err(invalid("cache capacity must be 1..100000"));
        }
        Ok(Self {
            capacity,
            values: BTreeMap::new(),
            order: VecDeque::new(),
        })
    }
    pub fn get_or_embed(
        &mut self,
        provider: &mut dyn LocalEmbedder,
        text: &str,
    ) -> Result<Vec<f32>> {
        if text.len() > 65536 {
            return Err(invalid("embedding input exceeds 64 KiB"));
        }
        let key = id(&[provider.model().fingerprint(), &hash(text)]);
        if let Some(value) = self.values.get(&key) {
            return Ok(value.clone());
        }
        let mut value = provider.embed(text)?;
        if value.len() != provider.model().manifest().dimensions
            || value.iter().any(|v| !v.is_finite())
        {
            return Err(invalid(
                "model returned non-finite or wrong-dimension vector",
            ));
        }
        let norm = value
            .iter()
            .map(|v| (*v as f64).powi(2))
            .sum::<f64>()
            .sqrt();
        if norm <= f64::EPSILON {
            return Err(invalid("model returned a zero vector"));
        }
        for v in &mut value {
            *v = (*v as f64 / norm) as f32;
        }
        while self.values.len() >= self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.values.remove(&old);
            }
        }
        self.order.push_back(key.clone());
        self.values.insert(key, value.clone());
        Ok(value)
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankingWeights {
    pub lexical: f64,
    pub vector: f64,
    pub rank_constant: f64,
}
impl Default for RankingWeights {
    fn default() -> Self {
        Self {
            lexical: 1.0,
            vector: 1.0,
            rank_constant: 60.0,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct FusedHit {
    pub symbol_id: String,
    pub score: f64,
    pub lexical_rank: Option<usize>,
    pub vector_rank: Option<usize>,
}
/// Weighted reciprocal-rank fusion over independently ranked, real retrieval lists.
/// Empty vector list means lexical-only; it is never relabeled semantic search.
pub fn reciprocal_rank_fusion(
    lexical: &[String],
    vector: &[String],
    weights: RankingWeights,
) -> Result<Vec<FusedHit>> {
    if !weights.lexical.is_finite()
        || !weights.vector.is_finite()
        || !weights.rank_constant.is_finite()
        || weights.lexical < 0.0
        || weights.vector < 0.0
        || weights.lexical + weights.vector <= 0.0
        || !(1.0..=1000.0).contains(&weights.rank_constant)
        || lexical.len() > 100_000
        || vector.len() > 100_000
    {
        return Err(Error::new(
            "INVALID_RANKING",
            "nonnegative finite weights, k1..1000 and <=100000 candidates required",
            2,
        ));
    }
    let mut hits = BTreeMap::<String, FusedHit>::new();
    for (list, weight, is_lexical) in [
        (lexical, weights.lexical, true),
        (vector, weights.vector, false),
    ] {
        if weight == 0.0 {
            continue;
        }
        let mut seen = BTreeSet::new();
        for (rank, id) in list.iter().enumerate() {
            if !seen.insert(id) {
                continue;
            }
            let hit = hits.entry(id.clone()).or_insert(FusedHit {
                symbol_id: id.clone(),
                score: 0.0,
                lexical_rank: None,
                vector_rank: None,
            });
            hit.score += weight / (weights.rank_constant + (rank + 1) as f64);
            if is_lexical {
                hit.lexical_rank = Some(rank + 1);
            } else {
                hit.vector_rank = Some(rank + 1);
            }
        }
    }
    let mut hits = hits.into_values().collect::<Vec<_>>();
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.symbol_id.cmp(&b.symbol_id))
    });
    Ok(hits)
}
#[derive(Debug, Serialize)]
pub struct RelevanceMetrics {
    pub queries: usize,
    pub mrr_at_10: f64,
    pub ndcg_at_10: f64,
}
/// Ground truth must be curated relevance judgments, not self-labeled retrieval hits.
pub fn evaluate_relevance(
    rankings: &[Vec<String>],
    judgments: &[BTreeMap<String, u8>],
) -> Result<RelevanceMetrics> {
    if rankings.is_empty()
        || rankings.len() != judgments.len()
        || judgments.iter().any(|j| j.values().any(|v| *v > 3))
    {
        return Err(Error::new(
            "INVALID_EVALUATION",
            "nonempty paired queries and graded judgments 0..3 required",
            2,
        ));
    }
    let mut reciprocal = 0.0;
    let mut normalized = 0.0;
    for (ranking, labels) in rankings.iter().zip(judgments) {
        let mut seen = BTreeSet::new();
        let unique = ranking
            .iter()
            .filter(|s| seen.insert(*s))
            .take(10)
            .collect::<Vec<_>>();
        if let Some(rank) = unique
            .iter()
            .position(|s| labels.get(*s).copied().unwrap_or(0) > 0)
        {
            reciprocal += 1.0 / (rank + 1) as f64;
        }
        let gain = |grade: u8, rank: usize| {
            ((2_u32.pow(grade as u32) - 1) as f64) / ((rank + 2) as f64).log2()
        };
        let dcg = unique
            .iter()
            .enumerate()
            .map(|(i, s)| gain(labels.get(*s).copied().unwrap_or(0), i))
            .sum::<f64>();
        let mut ideal = labels.values().copied().collect::<Vec<_>>();
        ideal.sort_by(|a, b| b.cmp(a));
        let idcg = ideal
            .into_iter()
            .take(10)
            .enumerate()
            .map(|(i, g)| gain(g, i))
            .sum::<f64>();
        normalized += if idcg > 0.0 { dcg / idcg } else { 0.0 };
    }
    Ok(RelevanceMetrics {
        queries: rankings.len(),
        mrr_at_10: reciprocal / rankings.len() as f64,
        ndcg_at_10: normalized / rankings.len() as f64,
    })
}
/// Filename convenience only; no artifact acquisition or model execution occurs.
pub fn manifest_artifact(path: &Path, checksum: &str) -> Artifact {
    Artifact {
        path: path.into(),
        blake3: checksum.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rrf_weights_duplicates_and_curated_metric_arithmetic() {
        let names = |s: &[&str]| s.iter().map(|s| (*s).into()).collect::<Vec<String>>();
        let fused = reciprocal_rank_fusion(
            &names(&["a", "b", "b"]),
            &names(&["b", "c"]),
            RankingWeights::default(),
        )
        .unwrap();
        assert_eq!(fused[0].symbol_id, "b");
        assert!((fused[0].score - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-12);
        let lexical = reciprocal_rank_fusion(
            &names(&["a", "b"]),
            &names(&["b"]),
            RankingWeights {
                vector: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(lexical[0].symbol_id, "a");
        assert!(
            reciprocal_rank_fusion(
                &[],
                &[],
                RankingWeights {
                    vector: f64::NAN,
                    ..Default::default()
                }
            )
            .is_err()
        );
        let judgments = vec![BTreeMap::from([("b".into(), 3), ("a".into(), 1)])];
        let metrics = evaluate_relevance(&[names(&["a", "b"])], &judgments).unwrap();
        assert_eq!(metrics.mrr_at_10, 1.0);
        assert!(metrics.ndcg_at_10 < 1.0 && metrics.ndcg_at_10 > 0.7);
        let perfect = evaluate_relevance(&[names(&["b", "a"])], &judgments).unwrap();
        assert_eq!(perfect.ndcg_at_10, 1.0);
    }
    #[test]
    fn explicit_artifacts_reject_corruption_and_version_drift() {
        let temp = tempfile::tempdir().unwrap();
        let model = temp.path().join("model.safetensors");
        let tokenizer = temp.path().join("tokenizer.json");
        std::fs::write(&model, b"artifact-verification-test-not-a-model").unwrap();
        std::fs::write(&tokenizer, b"{}").unwrap();
        let manifest = LocalModelManifest {
            schema_version: "1.0".into(),
            model_id: "verification-fixture".into(),
            revision: "fixture-v1".into(),
            license: "test-fixture-not-a-distributed-model".into(),
            architecture: "fixture-no-inference".into(),
            model: manifest_artifact(&model, &hash(b"artifact-verification-test-not-a-model")),
            tokenizer: manifest_artifact(&tokenizer, &hash(b"{}")),
            config: manifest_artifact(&tokenizer, &hash(b"{}")),
            dimensions: 3,
            max_tokens: 128,
            pooling: "attention_mean_l2".into(),
        };
        let verified = verify_model(manifest.clone()).unwrap();
        assert!(!verified.fingerprint().is_empty());
        let mut updated = manifest.clone();
        updated.revision = "fixture-v2".into();
        assert_ne!(
            verified.fingerprint(),
            verify_model(updated).unwrap().fingerprint()
        );
        std::fs::write(&model, b"corrupt").unwrap();
        assert!(verify_model(manifest).is_err());
    }
}
