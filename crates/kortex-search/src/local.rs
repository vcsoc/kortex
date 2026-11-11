//! Native CPU BERT/MiniLM inference over explicitly supplied immutable artifacts.
//! No pickle, Python, hub client, network transport, custom model code or dynamic plugins.
use crate::hybrid::{Artifact, LocalEmbedder, LocalModelManifest, VerifiedModel, verify_model};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use kortex_core::{Error, Result, hash};
use std::{fs::File, io::Read};
use tokenizers::{Tokenizer, TruncationParams};

pub struct NativeBert {
    identity: VerifiedModel,
    encoder: BertModel,
    tokenizer: Tokenizer,
    dimensions: usize,
}
fn invalid(message: impl std::fmt::Display) -> Error {
    Error::new("INVALID_LOCAL_MODEL", message.to_string(), 2)
}
fn inference(error: impl std::fmt::Display) -> Error {
    Error::new("LOCAL_INFERENCE", error.to_string(), 7)
}
// Rehash exactly the bytes consumed by the decoder, not a previous path lookup.
fn artifact_bytes(artifact: &Artifact, max: u64) -> Result<Vec<u8>> {
    let file = File::open(&artifact.path)?;
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
        let config_bytes = artifact_bytes(&manifest.config, 1024 * 1024)?;
        let config: Config = serde_json::from_slice(&config_bytes).map_err(invalid)?;
        if config.model_type.as_deref() != Some("bert")
            || config.hidden_size != manifest.dimensions
            || !(1..=1024).contains(&config.hidden_size)
            || !(1..=12).contains(&config.num_hidden_layers)
            || !(1..=32).contains(&config.num_attention_heads)
            || !config
                .hidden_size
                .is_multiple_of(config.num_attention_heads)
            || !(1..=4096).contains(&config.intermediate_size)
            || !(1..=65536).contains(&config.vocab_size)
            || !(1..=512).contains(&config.max_position_embeddings)
            || !(1..=4).contains(&config.type_vocab_size)
            || config.pad_token_id >= config.vocab_size
            || !(2..=512).contains(&manifest.max_tokens)
            || manifest.max_tokens > config.max_position_embeddings
            || !config.layer_norm_eps.is_finite()
            || config.layer_norm_eps <= 0.0
        {
            return Err(invalid(
                "BERT config dimensions/token bounds exceed native CPU policy or disagree with manifest",
            ));
        }
        let tokenizer_bytes = artifact_bytes(&manifest.tokenizer, 32 * 1024 * 1024)?;
        let specification: serde_json::Value =
            serde_json::from_slice(&tokenizer_bytes).map_err(invalid)?;
        // Restrict the supplied tokenizer to fixed BERT components, not arbitrary regex programs.
        if specification["model"]["type"] != "WordPiece"
            || specification["normalizer"]["type"] != "BertNormalizer"
            || specification["pre_tokenizer"]["type"] != "BertPreTokenizer"
            || specification["post_processor"]["type"] != "TemplateProcessing"
        {
            return Err(invalid(
                "native adapter requires WordPiece/BertNormalizer/BertPreTokenizer/TemplateProcessing tokenizer",
            ));
        }
        let mut tokenizer = Tokenizer::from_bytes(&tokenizer_bytes).map_err(invalid)?;
        tokenizer.with_padding(None);
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: manifest.max_tokens,
                ..Default::default()
            }))
            .map_err(invalid)?;
        if tokenizer.get_vocab_size(true) > config.vocab_size {
            return Err(invalid("tokenizer vocabulary exceeds model vocabulary"));
        }
        let weights = artifact_bytes(&manifest.model, 512 * 1024 * 1024)?;
        let builder = VarBuilder::from_buffered_safetensors(weights, DType::F32, &Device::Cpu)
            .map_err(inference)?;
        let encoder = BertModel::load(builder, &config).map_err(inference)?;
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
    fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() || text.len() > 65536 {
            return Err(invalid("embedding text must be nonempty and <=64 KiB"));
        }
        let tokens = self.tokenizer.encode(text, true).map_err(inference)?;
        let count = tokens.len();
        if count == 0 || count > self.identity.manifest().max_tokens {
            return Err(invalid("tokenizer violated sequence bound"));
        }
        let ids = Tensor::new(tokens.get_ids(), &Device::Cpu)
            .and_then(|t| t.unsqueeze(0))
            .map_err(inference)?;
        let types = Tensor::new(tokens.get_type_ids(), &Device::Cpu)
            .and_then(|t| t.unsqueeze(0))
            .map_err(inference)?;
        let mask = Tensor::new(tokens.get_attention_mask(), &Device::Cpu)
            .and_then(|t| t.unsqueeze(0))
            .map_err(inference)?;
        let states = self
            .encoder
            .forward(&ids, &types, Some(&mask))
            .and_then(|t| t.to_vec3::<f32>())
            .map_err(inference)?;
        let states = states
            .first()
            .ok_or_else(|| inference("missing encoder batch"))?;
        let mut vector = vec![0.0_f64; self.dimensions];
        let mut denominator = 0.0;
        for (index, state) in states.iter().enumerate() {
            let weight = if self.identity.manifest().pooling == "cls_l2" {
                u32::from(index == 0)
            } else {
                tokens.get_attention_mask()[index]
            } as f64;
            denominator += weight;
            for (out, value) in vector.iter_mut().zip(state) {
                *out += weight * (*value as f64);
            }
        }
        if denominator <= 0.0 {
            return Err(inference("all encoder tokens are masked"));
        }
        for value in &mut vector {
            *value /= denominator;
        }
        let norm = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !norm.is_finite() || norm <= f64::EPSILON {
            return Err(inference("encoder returned a non-finite or zero embedding"));
        }
        Ok(vector.into_iter().map(|v| (v / norm) as f32).collect())
    }
}

#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;
