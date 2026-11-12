//! Real shared-service integration using generated, nonpretrained BERT weights.
use super::*;
use kortex_search::{
    hybrid::{Artifact, LocalModelManifest},
    local::NativeBert,
};
use safetensors::{Dtype, tensor::TensorView};
use std::fs;

#[test]
fn explicit_native_service_query_preserves_freshness_schema_filters_cursors_and_budgets() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("app.py"),
        "def hello():\n    pass\ndef world():\n    hello()\n",
    )
    .unwrap();
    let mut config = Config::default();
    kortex_indexer::index_at_root(&root, &config, &Default::default()).unwrap();
    let mut shapes = vec![
        ("embeddings.word_embeddings.weight".to_owned(), vec![8, 4]),
        ("embeddings.position_embeddings.weight".into(), vec![16, 4]),
        ("embeddings.token_type_embeddings.weight".into(), vec![2, 4]),
        ("embeddings.LayerNorm.weight".into(), vec![4]),
        ("embeddings.LayerNorm.bias".into(), vec![4]),
    ];
    for (name, from, to) in [
        ("attention.self.query", 4, 4),
        ("attention.self.key", 4, 4),
        ("attention.self.value", 4, 4),
        ("attention.output.dense", 4, 4),
        ("intermediate.dense", 4, 8),
        ("output.dense", 8, 4),
    ] {
        shapes.push((format!("encoder.layer.0.{name}.weight"), vec![to, from]));
        shapes.push((format!("encoder.layer.0.{name}.bias"), vec![to]));
    }
    for name in ["attention.output.LayerNorm", "output.LayerNorm"] {
        for kind in ["weight", "bias"] {
            shapes.push((format!("encoder.layer.0.{name}.{kind}"), vec![4]));
        }
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
    let model = temp.path().join("model.safetensors");
    fs::write(&model, safetensors::serialize(views, None).unwrap()).unwrap();
    let encoder_config = temp.path().join("config.json");
    fs::write(&encoder_config,serde_json::to_vec(&json!({"model_type":"bert","vocab_size":8,"hidden_size":4,"num_hidden_layers":1,"num_attention_heads":2,"intermediate_size":8,"max_position_embeddings":16,"type_vocab_size":2,"layer_norm_eps":1e-12,"pad_token_id":0,"hidden_act":"gelu"})).unwrap()).unwrap();
    let tokenizer = temp.path().join("tokenizer.json");
    fs::write(&tokenizer,serde_json::to_vec(&json!({"version":"1.0","added_tokens":[],"model":{"type":"WordPiece","unk_token":"[UNK]","continuing_subword_prefix":"##","max_input_chars_per_word":100,"vocab":{"[PAD]":0,"[UNK]":1,"[CLS]":2,"[SEP]":3,"hello":4,"world":5,"code":6,"search":7}},"normalizer":{"type":"BertNormalizer","clean_text":true,"handle_chinese_chars":true,"strip_accents":null,"lowercase":true},"pre_tokenizer":{"type":"BertPreTokenizer"},"post_processor":{"type":"TemplateProcessing","single":[{"SpecialToken":{"id":"[CLS]","type_id":0}},{"Sequence":{"id":"A","type_id":0}},{"SpecialToken":{"id":"[SEP]","type_id":0}}],"special_tokens":{"[CLS]":{"id":"[CLS]","ids":[2],"tokens":["[CLS]"]},"[SEP]":{"id":"[SEP]","ids":[3],"tokens":["[SEP]"]}}}})).unwrap()).unwrap();
    let artifact = |path: PathBuf| Artifact {
        blake3: hash(fs::read(&path).unwrap()),
        path: path.canonicalize().unwrap(),
    };
    let manifest = LocalModelManifest {
        schema_version: "1.0".into(),
        model_id: "generated-not-pretrained".into(),
        revision: "fixture-1".into(),
        license: "private-test-fixture".into(),
        architecture: "bert".into(),
        model: artifact(model),
        tokenizer: artifact(tokenizer),
        config: artifact(encoder_config),
        dimensions: 4,
        max_tokens: 8,
        pooling: "attention_mean_l2".into(),
    };
    let manifest_path = temp.path().join("manifest.json");
    let bytes = serde_json::to_vec(&manifest).unwrap();
    fs::write(&manifest_path, &bytes).unwrap();
    config.search.embeddings = "local".into();
    config.search.model_path = manifest_path.to_str().unwrap().into();
    config.search.model_version = manifest.revision.clone();
    config.search.model_checksum = hash(bytes);
    config.search.cache_path = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("private-vectors")
        .to_str()
        .unwrap()
        .into();
    // Publish with the real effective configuration before requiring freshness.
    kortex_indexer::index_at_root(&root, &config, &Default::default()).unwrap();
    let mut service = Service::open_at_root(&root, config).unwrap();
    let mut provider = NativeBert::load(manifest.clone()).unwrap();
    let mut cache =
        kortex_search::persistence::configured_cache(&service.config.search, 100).unwrap();
    let args = json!({"text":"hello","kind":"function","limit":1,"require_fresh":true});
    assert_eq!(
        service.execute("query", &args).unwrap_err().code,
        "LOCAL_MODEL_GATE"
    );
    let first = service
        .execute_with_local_model(&args, &mut provider, &mut cache)
        .unwrap();
    assert_eq!(first["index_status"], "fresh");
    assert_eq!(
        first["data"]["retrieval"]["pretrained_quality_accepted"],
        false
    );
    assert_eq!(first["data"]["items"].as_array().unwrap().len(), 1);
    jsonschema::validator_for(&response_schema("query"))
        .unwrap()
        .validate(&first)
        .unwrap();
    let mut second_args = args.clone();
    second_args["cursor"] = first["page"]["next_cursor"].clone();
    let second = service
        .execute_with_local_model(&second_args, &mut provider, &mut cache)
        .unwrap();
    assert_ne!(
        first["data"]["items"][0]["symbol"]["symbol_id"],
        second["data"]["items"][0]["symbol"]["symbol_id"]
    );
    drop(cache);
    let mut cache =
        kortex_search::persistence::configured_cache(&service.config.search, 100).unwrap();
    let restarted = service
        .execute_with_local_model(&args, &mut provider, &mut cache)
        .unwrap();
    assert_eq!(restarted["data"], first["data"]);
    service.config.search.vector_weight = 0.25;
    assert_eq!(
        service
            .execute_with_local_model(&second_args, &mut provider, &mut cache)
            .unwrap_err()
            .code,
        "STALE_INDEX"
    );
    second_args.as_object_mut().unwrap().remove("require_fresh");
    assert_eq!(
        service
            .execute_with_local_model(&second_args, &mut provider, &mut cache)
            .unwrap_err()
            .code,
        "INVALID_CURSOR"
    );
    service.config.search.vector_weight = 1.0;
    let error = service
        .execute_with_local_model(
            &json!({"text":"hello","max_response_bytes":1024}),
            &mut provider,
            &mut cache,
        )
        .unwrap_err();
    assert_eq!(error.code, "RESPONSE_BUDGET");
    fs::write(root.join("app.py"), "def changed():\n    pass\n").unwrap();
    assert_eq!(
        service
            .execute_with_local_model(&args, &mut provider, &mut cache)
            .unwrap_err()
            .code,
        "STALE_INDEX"
    );
    service.config.search.model_version = "another-version".into();
    assert_eq!(
        service
            .execute_with_local_model(&args, &mut provider, &mut cache)
            .unwrap_err()
            .code,
        "INVALID_LOCAL_MODEL"
    );
}
