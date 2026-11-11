//! These generated tensors exercise real native inference, NOT pretrained relevance.
use super::*;
#[path = "retrieval_tests.rs"]
mod retrieval_tests;
use crate::hybrid::{EmbeddingCache, manifest_artifact};
use candle_nn::VarMap;
use serde_json::json;
fn fixture() -> (tempfile::TempDir, LocalModelManifest) {
    let temp = tempfile::tempdir().unwrap();
    let config = json!({"model_type":"bert","vocab_size":8,"hidden_size":4,"num_hidden_layers":1,"num_attention_heads":2,"intermediate_size":8,"hidden_act":"gelu","hidden_dropout_prob":0.0,"max_position_embeddings":16,"type_vocab_size":2,"initializer_range":0.02,"layer_norm_eps":1e-12,"pad_token_id":0});
    let config_path = temp.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let config: Config = serde_json::from_value(config).unwrap();
    let weights = VarMap::new();
    let builder = VarBuilder::from_varmap(&weights, DType::F32, &Device::Cpu);
    let _model = BertModel::load(builder, &config).unwrap();
    for (name, var) in weights.data().lock().unwrap().iter() {
        let values = (0..var.elem_count())
            .map(|i| {
                if name.ends_with("LayerNorm.weight") {
                    1.0
                } else if name.ends_with("bias") {
                    0.0
                } else {
                    ((i * 7 + i / 4 + 3) % 19) as f32 / 50.0 - 0.15
                }
            })
            .collect::<Vec<_>>();
        var.set(&Tensor::from_vec(values, var.dims(), &Device::Cpu).unwrap())
            .unwrap();
    }
    let model_path = temp.path().join("model.safetensors");
    weights.save(&model_path).unwrap();
    let vocab = temp.path().join("vocab.txt");
    std::fs::write(
        &vocab,
        "[PAD]\n[UNK]\n[CLS]\n[SEP]\nhello\nworld\ncode\nsearch\n",
    )
    .unwrap();
    let wordpiece = tokenizers::models::wordpiece::WordPiece::from_file(vocab.to_str().unwrap())
        .build()
        .unwrap();
    let mut tokenizer = Tokenizer::new(wordpiece);
    tokenizer.with_normalizer(Some(
        tokenizers::normalizers::bert::BertNormalizer::default(),
    ));
    tokenizer.with_pre_tokenizer(Some(tokenizers::pre_tokenizers::bert::BertPreTokenizer));
    tokenizer.with_post_processor(Some(
        tokenizers::processors::template::TemplateProcessing::builder()
            .try_single("[CLS] $A [SEP]")
            .unwrap()
            .special_tokens(vec![("[CLS]", 2), ("[SEP]", 3)])
            .build()
            .unwrap(),
    ));
    let tokenizer_path = temp.path().join("tokenizer.json");
    tokenizer.save(&tokenizer_path, false).unwrap();
    let artifact =
        |path: &std::path::Path| manifest_artifact(path, &hash(std::fs::read(path).unwrap()));
    let manifest = LocalModelManifest {
        schema_version: "1.0".into(),
        model_id: "generated-tiny-bert-inference-fixture-not-pretrained".into(),
        revision: "fixture-v1".into(),
        license: "private-test-fixture".into(),
        architecture: "bert".into(),
        model: artifact(&model_path),
        tokenizer: artifact(&tokenizer_path),
        config: artifact(&config_path),
        dimensions: 4,
        max_tokens: 8,
        pooling: "attention_mean_l2".into(),
    };
    (temp, manifest)
}
#[test]
fn native_cpu_encoder_runs_supplied_weights_and_tokenizer_without_downloads() {
    let (_temp, manifest) = fixture();
    let mut model = NativeBert::load(manifest).unwrap();
    let a = model.embed("hello world").unwrap();
    let b = model.embed("code search").unwrap();
    assert_eq!(a.len(), 4);
    assert!(a.iter().all(|v| v.is_finite()));
    assert!((a.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1e-5);
    assert_ne!(
        a, b,
        "actual token-dependent encoder, not a fixed placeholder"
    );
    assert_eq!(a, model.embed("hello world").unwrap());
    assert!(model.embed("").is_err());
    assert!(model.embed(&"x".repeat(65537)).is_err());
    assert_eq!(
        model.embed(&"hello ".repeat(100)).unwrap().len(),
        4,
        "real tokenizer truncation respects supplied bound"
    );
    let mut cache = EmbeddingCache::new(2).unwrap();
    assert_eq!(
        cache.get_or_embed(&mut model, "hello world").unwrap(),
        cache.get_or_embed(&mut model, "hello world").unwrap()
    );
}
#[test]
fn native_model_rejects_corrupt_artifacts_wrong_config_and_tokenizer_programs() {
    let (_temp, mut manifest) = fixture();
    manifest.dimensions = 5;
    assert!(NativeBert::load(manifest).is_err());
    let (_temp, manifest) = fixture();
    std::fs::write(&manifest.model.path, b"not safetensors").unwrap();
    assert!(NativeBert::load(manifest).is_err());
    let (_temp, mut manifest) = fixture();
    let mut tokenizer: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest.tokenizer.path).unwrap()).unwrap();
    tokenizer["pre_tokenizer"] =
        json!({"type":"Split","pattern":{"Regex":"(a+)+$"},"behavior":"Removed","invert":false});
    let bytes = serde_json::to_vec(&tokenizer).unwrap();
    std::fs::write(&manifest.tokenizer.path, &bytes).unwrap();
    manifest.tokenizer.blake3 = hash(&bytes);
    assert!(NativeBert::load(manifest).is_err());
}
