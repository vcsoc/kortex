//! Fixed native CPU BERT over explicitly supplied safetensors, using maintained tract.
//! No supplied ONNX graph, pickle, hub client, Python, network or custom model code.
use crate::hybrid::{Artifact, LocalEmbedder, LocalModelManifest, VerifiedModel, verify_model};
use kortex_core::{Error, Result, hash};
use serde::Deserialize;
use std::io::Read;
#[path = "bert_graph.rs"]
mod bert_graph;
#[path = "bert_tokenizer.rs"]
mod bert_tokenizer;
use bert_graph::Encoder;
use bert_tokenizer::BertTokenizer;

pub struct NativeBert {
    identity: VerifiedModel,
    encoder: Encoder,
    tokenizer: BertTokenizer,
    dimensions: usize,
}
#[derive(Deserialize)]
pub(super) struct BertConfig {
    model_type: String,
    vocab_size: usize,
    hidden_size: usize,
    num_hidden_layers: usize,
    num_attention_heads: usize,
    intermediate_size: usize,
    max_position_embeddings: usize,
    type_vocab_size: usize,
    layer_norm_eps: f32,
    pad_token_id: usize,
    hidden_act: String,
    #[serde(default)]
    is_decoder: bool,
    #[serde(default)]
    add_cross_attention: bool,
    #[serde(default = "absolute")]
    position_embedding_type: String,
}
fn absolute() -> String {
    "absolute".into()
}
fn invalid(message: impl std::fmt::Display) -> Error {
    Error::new("INVALID_LOCAL_MODEL", message.to_string(), 2)
}
fn inference(error: impl std::fmt::Display) -> Error {
    Error::new("LOCAL_INFERENCE", error.to_string(), 7)
}
// Rehash the exact bytes consumed, not only an earlier path lookup.
fn artifact_bytes(artifact: &Artifact, max: u64) -> Result<Vec<u8>> {
    let file = crate::hybrid::open_artifact(&artifact.path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("artifact must be a regular file"));
    }
    let mut bytes = vec![];
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max || hash(&bytes) != artifact.blake3 {
        return Err(invalid(
            "artifact changed, oversized or checksum mismatch during model load",
        ));
    }
    Ok(bytes)
}
impl NativeBert {
    pub fn load(manifest: LocalModelManifest) -> Result<Self> {
        let identity = verify_model(manifest)?;
        let manifest = identity.manifest();
        if manifest.architecture != "bert" {
            return Err(invalid("native adapter supports architecture bert only"));
        }
        let config: BertConfig =
            serde_json::from_slice(&artifact_bytes(&manifest.config, 1024 * 1024)?)
                .map_err(invalid)?;
        if config.model_type != "bert"
            || config.hidden_size != manifest.dimensions
            || !(1..=1024).contains(&config.hidden_size)
            || !(1..=12).contains(&config.num_hidden_layers)
            || !(1..=32).contains(&config.num_attention_heads)
            || !config
                .hidden_size
                .is_multiple_of(config.num_attention_heads)
            || !(1..=4096).contains(&config.intermediate_size)
            || !(4..=65536).contains(&config.vocab_size)
            || !(2..=512).contains(&config.max_position_embeddings)
            || !(1..=4).contains(&config.type_vocab_size)
            || config.pad_token_id >= config.vocab_size
            || !(2..=512).contains(&manifest.max_tokens)
            || manifest.max_tokens > config.max_position_embeddings
            || !config.layer_norm_eps.is_finite()
            || config.layer_norm_eps <= 0.0
            || config.hidden_act != "gelu"
            || config.is_decoder
            || config.add_cross_attention
            || config.position_embedding_type != "absolute"
        {
            return Err(invalid(
                "unsupported BERT architecture or dimensions/token bounds disagree with manifest",
            ));
        }
        // Aggregate work bound, not just individual dimensions. MiniLM-L6/256 fits.
        let n = manifest.max_tokens as u64;
        let h = config.hidden_size as u64;
        let work = config.num_hidden_layers as u64
            * (4 * n * h * h + 2 * n * n * h + 2 * n * h * config.intermediate_size as u64);
        if work > 4_000_000_000 {
            return Err(invalid("BERT CPU operation bound exceeded"));
        }
        let tokenizer = BertTokenizer::load(
            &artifact_bytes(&manifest.tokenizer, 32 * 1024 * 1024)?,
            config.vocab_size,
            config.pad_token_id,
        )?;
        let weights = artifact_bytes(&manifest.model, 512 * 1024 * 1024)?;
        let encoder = Encoder::load(&weights, &config, manifest.max_tokens)?;
        let dimensions = config.hidden_size;
        Ok(Self {
            identity,
            encoder,
            tokenizer,
            dimensions,
        })
    }
}
impl LocalEmbedder for NativeBert {
    fn model(&self) -> &VerifiedModel {
        &self.identity
    }
    fn cache_namespace(&self) -> &str {
        "kortex-bert-f32-tract-0.23.10-wordpiece-v1"
    }
    fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() || text.len() > 65536 {
            return Err(invalid("embedding text must be nonempty and <=64 KiB"));
        }
        let mut ids = self
            .tokenizer
            .encode(text, self.identity.manifest().max_tokens);
        let count = ids.len();
        ids.resize(self.identity.manifest().max_tokens, self.tokenizer.pad);
        let states = self.encoder.forward(ids, count)?;
        if states.len() != count || states.iter().any(|s| s.len() != self.dimensions) {
            return Err(inference("encoder returned invalid shape"));
        }
        let mut vector = vec![0.0_f64; self.dimensions];
        let cls = self.identity.manifest().pooling == "cls_l2";
        for state in states.iter().take(if cls { 1 } else { count }) {
            for (out, value) in vector.iter_mut().zip(state) {
                *out += *value as f64;
            }
        }
        // Mean's positive scalar cancels in L2 normalization; masked padding is never included.
        let norm = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !norm.is_finite() || norm <= f64::EPSILON {
            return Err(inference("encoder returned a nonfinite or zero embedding"));
        }
        Ok(vector.into_iter().map(|v| (v / norm) as f32).collect())
    }
}
#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;
