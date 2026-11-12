//! Fixed BERT encoder graph constructed by trusted code, never deserialized from a model artifact.
use super::{BertConfig, inference, invalid};
use kortex_core::Result;
use safetensors::{Dtype, SafeTensors};
use tract_onnx::{pb::*, prelude::*};

pub(super) struct Encoder(pub std::sync::Arc<TypedRunnableModel>);
struct Graph<'a> {
    nodes: Vec<NodeProto>,
    constants: Vec<TensorProto>,
    tensors: SafeTensors<'a>,
    prefix: &'static str,
}
impl Graph<'_> {
    fn op(&mut self, op: &str, inputs: &[&str], attributes: Vec<AttributeProto>) -> String {
        let name = format!("kx_{}", self.nodes.len());
        self.nodes.push(NodeProto {
            name: name.clone(),
            op_type: op.into(),
            input: inputs.iter().map(|s| (*s).into()).collect(),
            output: vec![name.clone()],
            attribute: attributes,
            ..Default::default()
        });
        name
    }
    fn float(&mut self, name: &str, shape: &[usize], values: Vec<f32>) -> String {
        self.constants.push(TensorProto {
            name: name.into(),
            data_type: tensor_proto::DataType::Float as i32,
            dims: shape.iter().map(|n| *n as i64).collect(),
            float_data: values,
            ..Default::default()
        });
        name.into()
    }
    fn ints(&mut self, name: &str, values: Vec<i64>) -> String {
        self.constants.push(TensorProto {
            name: name.into(),
            data_type: tensor_proto::DataType::Int64 as i32,
            dims: vec![values.len() as i64],
            int64_data: values,
            ..Default::default()
        });
        name.into()
    }
    fn weight(&mut self, name: &str, shape: &[usize], transpose: bool) -> Result<String> {
        let tensor = self
            .tensors
            .tensor(&format!("{}{name}", self.prefix))
            .map_err(invalid)?;
        if tensor.dtype() != Dtype::F32 || tensor.shape() != shape {
            return Err(invalid(format!(
                "expected F32 weight {name} with shape {shape:?}"
            )));
        }
        let values: Vec<f32> = tensor
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        if values.iter().any(|v| !v.is_finite()) {
            return Err(invalid("nonfinite model weight"));
        }
        if transpose {
            let values = (0..shape[1])
                .flat_map(|column| (0..shape[0]).map(move |row| (row, column)))
                .map(|(row, column)| values[row * shape[1] + column])
                .collect();
            Ok(self.float(name, &[shape[1], shape[0]], values))
        } else {
            Ok(self.float(name, shape, values))
        }
    }
    fn linear(&mut self, input: &str, name: &str, from: usize, to: usize) -> Result<String> {
        let weight = self.weight(&format!("{name}.weight"), &[to, from], true)?;
        let bias = self.weight(&format!("{name}.bias"), &[to], false)?;
        let product = self.op("MatMul", &[input, &weight], vec![]);
        Ok(self.op("Add", &[&product, &bias], vec![]))
    }
    fn norm(&mut self, input: &str, name: &str, hidden: usize) -> Result<String> {
        let mean = self.op(
            "ReduceMean",
            &[input],
            vec![ints_attr("axes", &[-1]), int_attr("keepdims", 1)],
        );
        let centered = self.op("Sub", &[input, &mean], vec![]);
        let square = self.op("Mul", &[&centered, &centered], vec![]);
        let variance = self.op(
            "ReduceMean",
            &[&square],
            vec![ints_attr("axes", &[-1]), int_attr("keepdims", 1)],
        );
        let stabilized = self.op("Add", &[&variance, "epsilon"], vec![]);
        let sd = self.op("Sqrt", &[&stabilized], vec![]);
        let normalized = self.op("Div", &[&centered, &sd], vec![]);
        let weight = self.weight(&format!("{name}.weight"), &[hidden], false)?;
        let bias = self.weight(&format!("{name}.bias"), &[hidden], false)?;
        let scaled = self.op("Mul", &[&normalized, &weight], vec![]);
        Ok(self.op("Add", &[&scaled, &bias], vec![]))
    }
}
fn int_attr(name: &str, value: i64) -> AttributeProto {
    AttributeProto {
        name: name.into(),
        r#type: attribute_proto::AttributeType::Int as i32,
        i: value,
        ..Default::default()
    }
}
fn ints_attr(name: &str, values: &[i64]) -> AttributeProto {
    AttributeProto {
        name: name.into(),
        r#type: attribute_proto::AttributeType::Ints as i32,
        ints: values.into(),
        ..Default::default()
    }
}
fn info(name: &str, dtype: tensor_proto::DataType, shape: &[usize]) -> ValueInfoProto {
    ValueInfoProto {
        name: name.into(),
        r#type: Some(TypeProto {
            value: Some(type_proto::Value::TensorType(type_proto::Tensor {
                elem_type: dtype as i32,
                shape: Some(TensorShapeProto {
                    dim: shape
                        .iter()
                        .map(|n| tensor_shape_proto::Dimension {
                            value: Some(tensor_shape_proto::dimension::Value::DimValue(*n as i64)),
                            ..Default::default()
                        })
                        .collect(),
                }),
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}
impl Encoder {
    pub fn load(bytes: &[u8], config: &BertConfig, sequence: usize) -> Result<Self> {
        let tensors = SafeTensors::deserialize(bytes).map_err(invalid)?;
        let prefix = if tensors.tensor("embeddings.word_embeddings.weight").is_ok() {
            ""
        } else {
            "bert."
        };
        let mut graph = Graph {
            nodes: vec![],
            constants: vec![],
            tensors,
            prefix,
        };
        let hidden = config.hidden_size;
        let heads = config.num_attention_heads;
        graph.float("epsilon", &[], vec![config.layer_norm_eps]);
        graph.float("scale", &[], vec![(hidden as f32 / heads as f32).sqrt()]);
        graph.float("sqrt2", &[], vec![std::f32::consts::SQRT_2]);
        graph.float("one", &[], vec![1.0]);
        graph.float("half", &[], vec![0.5]);
        graph.ints("positions", (0..sequence as i64).collect());
        graph.ints("types", vec![0; sequence]);
        graph.ints(
            "head_shape",
            vec![sequence as i64, heads as i64, (hidden / heads) as i64],
        );
        graph.ints("hidden_shape", vec![sequence as i64, hidden as i64]);
        let word = graph.weight(
            "embeddings.word_embeddings.weight",
            &[config.vocab_size, hidden],
            false,
        )?;
        let position = graph.weight(
            "embeddings.position_embeddings.weight",
            &[config.max_position_embeddings, hidden],
            false,
        )?;
        let types = graph.weight(
            "embeddings.token_type_embeddings.weight",
            &[config.type_vocab_size, hidden],
            false,
        )?;
        let word = graph.op("Gather", &[&word, "input_ids"], vec![int_attr("axis", 0)]);
        let position = graph.op(
            "Gather",
            &[&position, "positions"],
            vec![int_attr("axis", 0)],
        );
        let types = graph.op("Gather", &[&types, "types"], vec![int_attr("axis", 0)]);
        let sum = graph.op("Add", &[&word, &position], vec![]);
        let sum = graph.op("Add", &[&sum, &types], vec![]);
        let mut states = graph.norm(&sum, "embeddings.LayerNorm", hidden)?;
        for layer in 0..config.num_hidden_layers {
            let base = format!("encoder.layer.{layer}");
            let mut qkv = vec![];
            for (name, permutation) in [
                ("query", vec![1, 0, 2]),
                ("key", vec![1, 2, 0]),
                ("value", vec![1, 0, 2]),
            ] {
                let linear = graph.linear(
                    &states,
                    &format!("{base}.attention.self.{name}"),
                    hidden,
                    hidden,
                )?;
                let reshaped = graph.op("Reshape", &[&linear, "head_shape"], vec![]);
                qkv.push(graph.op(
                    "Transpose",
                    &[&reshaped],
                    vec![ints_attr("perm", &permutation)],
                ));
            }
            let scores = graph.op("MatMul", &[&qkv[0], &qkv[1]], vec![]);
            let scaled = graph.op("Div", &[&scores, "scale"], vec![]);
            let masked = graph.op("Add", &[&scaled, "attention_bias"], vec![]);
            let probability = graph.op("Softmax", &[&masked], vec![int_attr("axis", -1)]);
            let attention = graph.op("MatMul", &[&probability, &qkv[2]], vec![]);
            let attention = graph.op(
                "Transpose",
                &[&attention],
                vec![ints_attr("perm", &[1, 0, 2])],
            );
            let attention = graph.op("Reshape", &[&attention, "hidden_shape"], vec![]);
            let attention = graph.linear(
                &attention,
                &format!("{base}.attention.output.dense"),
                hidden,
                hidden,
            )?;
            let residual = graph.op("Add", &[&attention, &states], vec![]);
            let attention = graph.norm(
                &residual,
                &format!("{base}.attention.output.LayerNorm"),
                hidden,
            )?;
            let intermediate = graph.linear(
                &attention,
                &format!("{base}.intermediate.dense"),
                hidden,
                config.intermediate_size,
            )?;
            let divided = graph.op("Div", &[&intermediate, "sqrt2"], vec![]);
            let erf = graph.op("Erf", &[&divided], vec![]);
            let shifted = graph.op("Add", &[&erf, "one"], vec![]);
            let product = graph.op("Mul", &[&intermediate, &shifted], vec![]);
            let gelu = graph.op("Mul", &[&product, "half"], vec![]);
            let output = graph.linear(
                &gelu,
                &format!("{base}.output.dense"),
                config.intermediate_size,
                hidden,
            )?;
            let residual = graph.op("Add", &[&output, &attention], vec![]);
            states = graph.norm(&residual, &format!("{base}.output.LayerNorm"), hidden)?;
        }
        let model = ModelProto {
            ir_version: 8,
            opset_import: vec![OperatorSetIdProto {
                domain: "".into(),
                version: 12,
            }],
            graph: Some(GraphProto {
                name: "kortex-fixed-bert-v1".into(),
                node: graph.nodes,
                initializer: graph.constants,
                input: vec![
                    info("input_ids", tensor_proto::DataType::Int64, &[sequence]),
                    info(
                        "attention_bias",
                        tensor_proto::DataType::Float,
                        &[1, 1, sequence],
                    ),
                ],
                output: vec![info(
                    &states,
                    tensor_proto::DataType::Float,
                    &[sequence, hidden],
                )],
                ..Default::default()
            }),
            ..Default::default()
        };
        let plan = tract_onnx::onnx()
            .model_for_proto_model(&model)
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(inference)?;
        Ok(Self(plan))
    }
    pub fn forward(&self, ids: Vec<i64>, count: usize) -> Result<Vec<Vec<f32>>> {
        let sequence = ids.len();
        let mask = (0..sequence)
            .map(|i| if i < count { 0.0_f32 } else { -10000.0 })
            .collect::<Vec<_>>();
        let ids = Tensor::from_shape(&[sequence], &ids).map_err(inference)?;
        let mask = Tensor::from_shape(&[1, 1, sequence], &mask).map_err(inference)?;
        let outputs = self
            .0
            .run(tvec!(ids.into(), mask.into()))
            .map_err(inference)?;
        let states = outputs[0].to_plain_array_view::<f32>().map_err(inference)?;
        Ok(states
            .outer_iter()
            .take(count)
            .map(|row| row.iter().copied().collect())
            .collect())
    }
}
