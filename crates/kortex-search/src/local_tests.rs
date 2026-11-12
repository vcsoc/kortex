//! These generated tensors exercise real native inference, NOT pretrained relevance.
use super::*;
#[path = "persistence_native_tests.rs"]
mod persistence_native_tests;
#[path = "retrieval_tests.rs"]
mod retrieval_tests;
use crate::hybrid::{EmbeddingCache, manifest_artifact};
use safetensors::{Dtype, tensor::TensorView};
use serde_json::json;
fn fixture() -> (tempfile::TempDir, LocalModelManifest) {
    let temp = tempfile::tempdir().unwrap();
    let config = json!({"model_type":"bert","vocab_size":8,"hidden_size":4,"num_hidden_layers":1,"num_attention_heads":2,"intermediate_size":8,"hidden_act":"gelu","hidden_dropout_prob":0.0,"max_position_embeddings":16,"type_vocab_size":2,"initializer_range":0.02,"layer_norm_eps":1e-12,"pad_token_id":0});
    let config_path = temp.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let mut shapes = vec![
        ("embeddings.word_embeddings.weight".to_owned(), vec![8, 4]),
        (
            "embeddings.position_embeddings.weight".to_owned(),
            vec![16, 4],
        ),
        (
            "embeddings.token_type_embeddings.weight".to_owned(),
            vec![2, 4],
        ),
        ("embeddings.LayerNorm.weight".to_owned(), vec![4]),
        ("embeddings.LayerNorm.bias".to_owned(), vec![4]),
    ];
    for (part, from, to) in [
        ("attention.self.query", 4, 4),
        ("attention.self.key", 4, 4),
        ("attention.self.value", 4, 4),
        ("attention.output.dense", 4, 4),
        ("intermediate.dense", 4, 8),
        ("output.dense", 8, 4),
    ] {
        shapes.push((format!("encoder.layer.0.{part}.weight"), vec![to, from]));
        shapes.push((format!("encoder.layer.0.{part}.bias"), vec![to]));
    }
    for part in ["attention.output.LayerNorm", "output.LayerNorm"] {
        shapes.push((format!("encoder.layer.0.{part}.weight"), vec![4]));
        shapes.push((format!("encoder.layer.0.{part}.bias"), vec![4]));
    }
    let tensors: Vec<_> = shapes
        .into_iter()
        .map(|(name, shape)| {
            let bytes: Vec<_> = (0..shape.iter().product())
                .flat_map(|i: usize| {
                    let value: f32 = if name.ends_with("LayerNorm.weight") {
                        1.0
                    } else if name.ends_with("bias") {
                        0.0
                    } else {
                        ((i * 7 + i / 4 + 3) % 19) as f32 / 50.0 - 0.15
                    };
                    value.to_le_bytes()
                })
                .collect();
            (name, shape, bytes)
        })
        .collect();
    let views: Vec<_> = tensors
        .iter()
        .map(|(name, shape, bytes)| {
            (
                name.as_str(),
                TensorView::new(Dtype::F32, shape.clone(), bytes).unwrap(),
            )
        })
        .collect();
    let model_path = temp.path().join("model.safetensors");
    std::fs::write(&model_path, safetensors::serialize(views, None).unwrap()).unwrap();
    let tokenizer = json!({"version":"1.0","added_tokens":[],"model":{"type":"WordPiece","unk_token":"[UNK]","continuing_subword_prefix":"##","max_input_chars_per_word":100,"vocab":{"[PAD]":0,"[UNK]":1,"[CLS]":2,"[SEP]":3,"hello":4,"world":5,"code":6,"search":7}},"normalizer":{"type":"BertNormalizer","clean_text":true,"handle_chinese_chars":true,"strip_accents":null,"lowercase":true},"pre_tokenizer":{"type":"BertPreTokenizer"},"post_processor":{"type":"TemplateProcessing","single":[{"SpecialToken":{"id":"[CLS]","type_id":0}},{"Sequence":{"id":"A","type_id":0}},{"SpecialToken":{"id":"[SEP]","type_id":0}}],"special_tokens":{"[CLS]":{"id":"[CLS]","ids":[2],"tokens":["[CLS]"]},"[SEP]":{"id":"[SEP]","ids":[3],"tokens":["[SEP]"]}}}});
    let tokenizer_path = temp.path().join("tokenizer.json");
    std::fs::write(&tokenizer_path, serde_json::to_vec(&tokenizer).unwrap()).unwrap();
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
fn maintained_encoder_matches_independent_candle_reference_on_generated_weights() {
    // Captured from independently executed Candle 0.11.1 CPU BertModel using the same
    // explicit generated tensors, IDs [CLS,hello,world,SEP], and full attention mask.
    // Candle/tokenizers are evaluation-only references, NOT project dependencies.
    let (_temp, manifest) = fixture();
    let mut model = NativeBert::load(manifest).unwrap();
    let expected = [-0.5184145_f32, -0.41061503, 0.720536, 0.20849352];
    for (actual, expected) in model.embed("hello world").unwrap().iter().zip(expected) {
        assert!((actual - expected).abs() < 3e-5, "{actual} != {expected}");
    }
    let states = model
        .encoder
        .forward(vec![2, 4, 5, 3, 0, 0, 0, 0], 4)
        .unwrap();
    let expected = [
        [-1.0403557_f32, 0.28854477, 1.5054612, -0.7536503],
        [-1.344043, -0.43994117, 0.43994105, 1.3440431],
        [-0.848363, -0.6107537, 1.6894411, -0.23032434],
        [0.15820049, -1.6730847, 0.63844013, 0.87644416],
    ];
    for (actual, expected) in states.iter().flatten().zip(expected.iter().flatten()) {
        assert!((actual - expected).abs() < 3e-5, "{actual} != {expected}");
    }
    let without_padding = Encoder::load(
        &std::fs::read(&model.identity.manifest().model.path).unwrap(),
        &serde_json::from_slice(&std::fs::read(&model.identity.manifest().config.path).unwrap())
            .unwrap(),
        4,
    )
    .unwrap();
    let unpadded = without_padding.forward(vec![2, 4, 5, 3], 4).unwrap();
    for (a, b) in states.iter().flatten().zip(unpadded.iter().flatten()) {
        assert!(
            (a - b).abs() < 3e-5,
            "masked padding changed real-token state"
        );
    }
}
#[test]
fn validated_wordpiece_matches_independently_executed_stable_tokenizers_reference() {
    let (_temp, manifest) = fixture();
    let mut spec: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest.tokenizer.path).unwrap()).unwrap();
    for (index, word) in [
        "cafe", "中", "文", ",", "!", "un", "##aff", "##able", "é", "##🙂", "a", "b", "-",
        "[MASK]", ".", "##a", "istanbul",
    ]
    .iter()
    .enumerate()
    {
        spec["model"]["vocab"][*word] = json!(8 + index);
    }
    spec["added_tokens"] = json!([]);
    for (word, id) in [
        ("[PAD]", 0),
        ("[UNK]", 1),
        ("[CLS]", 2),
        ("[SEP]", 3),
        ("[MASK]", 21),
    ] {
        spec["added_tokens"].as_array_mut().unwrap().push(json!({"id":id,"content":word,"single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}));
    }
    let tokenizer = BertTokenizer::load(&serde_json::to_vec(&spec).unwrap(), 25, 0).unwrap();
    // Actual tokenizers 0.23.1 CPU output, not values derived from this implementation.
    for (text, expected) in [
        ("HÉLLO Café", vec![2, 4, 8, 3]),
        ("Cafe\u{301}!\0\u{7}\tWORLD", vec![2, 8, 12, 5, 3]),
        ("中文", vec![2, 9, 10, 3]),
        ("UNAFFABLE", vec![2, 13, 14, 15, 3]),
        ("[MASK] HELLO[SEP]world", vec![2, 21, 4, 3, 5, 3]),
        ("hello—world", vec![2, 4, 1, 5, 3]),
        ("İSTANBUL", vec![2, 24, 3]),
        ("é", vec![2, 1, 3]),
        ("a\u{200d}b", vec![2, 1, 3]),
        ("a🙂", vec![2, 18, 17, 3]),
        (
            "hello world code search hello world code",
            vec![2, 4, 5, 6, 7, 4, 5, 3],
        ),
        ("unknownword", vec![2, 1, 3]),
    ] {
        assert_eq!(tokenizer.encode(text, 8), expected, "{text:?}");
    }
    assert_eq!(tokenizer.encode(&"a".repeat(101), 8), vec![2, 1, 3]);
    for invalid in [
        ("/post_processor/single/0/SpecialToken/type_id", json!(1)),
        ("/model/vocab/hello", json!(1000)),
        ("/added_tokens/0/normalized", json!(true)),
        ("/model/continuing_subword_prefix", json!("CUSTOM")),
        ("/normalizer/type", json!("Sequence")),
    ] {
        let mut changed = spec.clone();
        *changed.pointer_mut(invalid.0).unwrap() = invalid.1;
        assert!(BertTokenizer::load(&serde_json::to_vec(&changed).unwrap(), 25, 0).is_err());
    }
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
fn checksummed_unsupported_architectures_and_nonfinite_tensors_are_rejected() {
    for (key, value) in [
        ("hidden_act", json!("gelu_new")),
        ("position_embedding_type", json!("relative_key")),
        ("is_decoder", json!(true)),
        ("add_cross_attention", json!(true)),
        ("num_attention_heads", json!(0)),
        ("layer_norm_eps", json!(0.0)),
    ] {
        let (_temp, mut manifest) = fixture();
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest.config.path).unwrap()).unwrap();
        config[key] = value;
        let bytes = serde_json::to_vec(&config).unwrap();
        std::fs::write(&manifest.config.path, &bytes).unwrap();
        manifest.config.blake3 = hash(&bytes);
        assert!(
            NativeBert::load(manifest).is_err(),
            "accepted unsupported {key}"
        );
    }
    let (_temp, mut manifest) = fixture();
    let mut bytes = std::fs::read(&manifest.model.path).unwrap();
    let end = bytes.len();
    bytes[end - 4..].copy_from_slice(&f32::NAN.to_le_bytes());
    std::fs::write(&manifest.model.path, &bytes).unwrap();
    manifest.model.blake3 = hash(&bytes);
    assert!(
        NativeBert::load(manifest).is_err(),
        "valid checksum must not make nonfinite weights acceptable"
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
