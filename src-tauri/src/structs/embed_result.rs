use crate::structs::sparse_vector::SparseVector;
use anyhow::Result;
use ort::value::Value;
use std::collections::HashMap;

pub struct EmbedResult {
    pub dense: Vec<f32>,
    pub sparse: SparseVector,
}

impl Default for EmbedResult {
    fn default() -> Self {
        EmbedResult {
            dense: Vec::new(),
            sparse: SparseVector {
                indices: Vec::new(),
                values: Vec::new(),
            },
        }
    }
}

impl EmbedResult {
    pub fn from_outputs(
        outputs: &ort::session::SessionOutputs<'_>,
        input_ids: &Vec<i64>,
    ) -> Result<Self> {
        let dense_vec = extract_dense_vector(&outputs[0])?;
        // BGE-M3 outputs: [dense_tensor, sparse_map_tensor]
        let sparse_vec = if outputs.len() > 1 {
            log::debug!("input_ids_len: {}", input_ids.len());
            let (_, s_data) = outputs[1].try_extract_tensor::<f32>()?;
            let limit = std::cmp::min(input_ids.len(), s_data.len());
            let mut sparse_map = HashMap::new();
            for i in 0..limit {
                let weight = s_data[i];
                let token_id = input_ids[i] as u32;
                // 1. Filter out items with low or zero weight
                // 2. Filter out special tokens ([CLS], [SEP], [PAD] in BGE-M3)
                //    typically PAD=0, CLS=0, SEP=2 (tokenizer-dependent)
                if weight > 0.0 && token_id != 0 {
                    let e = sparse_map.entry(token_id).or_insert(0.0f32);
                    if weight > *e {
                        *e = weight;
                    }
                }
            }
            log::debug!(
                "Extracted sparse vector with {} active tokens",
                sparse_map.len()
            );
            SparseVector::from_map(sparse_map)
        } else {
            SparseVector {
                indices: vec![],
                values: vec![],
            }
        };

        Ok(EmbedResult {
            dense: dense_vec,
            sparse: sparse_vec,
        })
    }

    pub fn get_sparse_blob(&self) -> Vec<u8> {
        let mut pairs: Vec<(u32, f32)> = self
            .sparse
            .indices
            .iter()
            .zip(self.sparse.values.iter())
            .map(|(&i, &v)| (i, v))
            .collect();
        pairs.sort_by_key(|p| p.0);
        let sparse_vec = SparseVector {
            indices: pairs.iter().map(|p| p.0).collect(),
            values: pairs.iter().map(|p| p.1).collect(),
        };
        sparse_vec.to_blob()
    }
}

fn extract_dense_vector(tensor_value: &Value) -> Result<Vec<f32>> {
    let (shape, data) = tensor_value.try_extract_tensor::<f32>()?;
    match shape
        .iter()
        .map(|&x| x as usize)
        .collect::<Vec<_>>()
        .as_slice()
    {
        &[1, seq_len, embed_dim] => {
            // Shape [1, 128, 384]
            // take the last sequence
            let start = ((seq_len - 1) * embed_dim) as usize;
            let end = (seq_len * embed_dim) as usize;
            Ok(data[start..end].to_vec())
        }
        &[1, embed_dim] => {
            // Shape [1, 384]
            Ok(data[..embed_dim].to_vec())
        }
        &[embed_dim] => {
            // Shape [384]
            Ok(data[..embed_dim].to_vec())
        }
        _ => Err(anyhow::anyhow!("Unexpected output shape: {:?}", shape)),
    }
}
