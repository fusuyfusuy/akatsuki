//! Pure Rust Vector embedding and semantic search engine powered by Hugging Face Candle.

use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};
#[cfg(feature = "vectors")]
use rusqlite::{params, Connection};

use crate::search::SearchHit;

pub fn get_models_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine user home directory")?;
    let dir = home.join(".cache/akatsuki/models/intfloat_multilingual-e5-small");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn are_models_available() -> bool {
    if let Ok(dir) = get_models_dir() {
        dir.join("tokenizer.json").is_file()
            && dir.join("config.json").is_file()
            && (dir.join("model.safetensors").is_file() || dir.join("pytorch_model.bin").is_file())
    } else {
        false
    }
}

pub fn setup_models() -> Result<String> {
    #[cfg(feature = "vectors")]
    {
        let target_dir = get_models_dir()?;
        println!("Fetching intfloat/multilingual-e5-small weights into: {}", target_dir.display());

        let api = hf_hub::api::sync::ApiBuilder::new()
            .with_cache_dir(target_dir.clone())
            .build()?;
        let repo = api.model("intfloat/multilingual-e5-small".to_string());

        let files = ["tokenizer.json", "config.json", "model.safetensors"];
        for f in files {
            println!("  Downloading {}...", f);
            let path = repo.get(f)?;
            let dest = target_dir.join(f);
            if path != dest {
                std::fs::copy(&path, &dest)?;
            }
        }

        Ok(format!("Successfully provisioned Candle model weights to {}", target_dir.display()))
    }
    #[cfg(not(feature = "vectors"))]
    {
        bail!("Vector features disabled. Build akatsuki with --features vectors to enable model setup.");
    }
}

#[cfg(feature = "vectors")]
pub mod candle_engine {
    use super::*;
    use candle_core::{Device, Tensor};
    use candle_transformers::models::bert::{BertModel, Config};
    use tokenizers::Tokenizer;

    pub struct CandleEmbedder {
        model: BertModel,
        tokenizer: Tokenizer,
        device: Device,
    }

    impl CandleEmbedder {
        pub fn load() -> Result<Self> {
            let model_dir = get_models_dir()?;
            let config_path = model_dir.join("config.json");
            let tokenizer_path = model_dir.join("tokenizer.json");
            let weights_path = model_dir.join("model.safetensors");

            if !weights_path.is_file() {
                bail!("Model weights not found at {}. Run 'akatsuki setup-models' first.", weights_path.display());
            }

            let device = Device::Cpu;
            let config_str = std::fs::read_to_string(&config_path)?;
            let config: Config = serde_json::from_str(&config_str)?;

            let tokenizer = Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

            let vb = unsafe {
                candle_nn::VarBuilder::from_mmaped_safetensors(&[weights_path], candle_core::DType::F32, &device)?
            };

            let model = BertModel::load(vb, &config)?;

            Ok(Self {
                model,
                tokenizer,
                device,
            })
        }

        pub fn encode_query(&self, text: &str) -> Result<Vec<f32>> {
            let prefixed = format!("query: {}", text.trim());
            self.encode_single(&prefixed)
        }

        pub fn encode_passage(&self, text: &str) -> Result<Vec<f32>> {
            let prefixed = format!("passage: {}", text.trim());
            self.encode_single(&prefixed)
        }

        fn encode_single(&self, input: &str) -> Result<Vec<f32>> {
            let encoding = self
                .tokenizer
                .encode(input, true)
                .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

            let tokens = encoding.get_ids();
            let token_type_ids = encoding.get_type_ids();
            let attention_mask = encoding.get_attention_mask();

            let token_tensor = Tensor::new(tokens, &self.device)?.unsqueeze(0)?;
            let token_type_tensor = Tensor::new(token_type_ids, &self.device)?.unsqueeze(0)?;
            let mask_tensor = Tensor::new(attention_mask, &self.device)?.unsqueeze(0)?;

            let output = self.model.forward(&token_tensor, &token_type_tensor, Some(&mask_tensor))?;

            // Mean pooling
            let mask_expanded = mask_tensor.unsqueeze(2)?.to_dtype(candle_core::DType::F32)?;
            let sum_embeddings = output.broadcast_mul(&mask_expanded)?.sum(1)?;
            let sum_mask = mask_expanded.sum(1)?.clamp(1e-9, f64::MAX)?;
            let mean_pooled = sum_embeddings.broadcast_div(&sum_mask)?;

            // L2 normalize
            let norm = mean_pooled.sqr()?.sum_keepdim(1)?.sqrt()?;
            let normalized = mean_pooled.broadcast_div(&norm)?;

            let vec: Vec<f32> = normalized.squeeze(0)?.to_vec1()?;
            Ok(vec)
        }
    }
}

pub fn search_vectors(
    vault: &Path,
    query: &str,
    domain_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    #[cfg(feature = "vectors")]
    {
        if !are_models_available() {
            return Ok(Vec::new());
        }

        let embedder = candle_engine::CandleEmbedder::load()?;
        let q_vec = embedder.encode_query(query)?;

        let db_path = vault.join(".akatsuki/cache.db");
        let con = Connection::open(&db_path)?;

        let domain_clause = if domain_filter.is_some() {
            "WHERE domain = ?1"
        } else {
            ""
        };

        let sql = format!(
            "SELECT chunk_id, rel_path, stem, domain, display_title, display_summary, breadcrumb, preview, vector_blob, dim FROM note_vectors {}",
            domain_clause
        );

        let mut stmt = con.prepare(&sql)?;
        let map_row = |r: &rusqlite::Row| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Vec<u8>>(8)?,
                r.get::<_, usize>(9)?,
            ))
        };

        let raw_rows: Vec<_> = if let Some(dom) = domain_filter {
            stmt.query_map(params![dom], map_row)?
                .filter_map(Result::ok)
                .collect()
        } else {
            stmt.query_map([], map_row)?
                .filter_map(Result::ok)
                .collect()
        };

        let mut hits = Vec::new();
        for r in raw_rows {
            let (_cid, rel, stem, domain, title, summary, breadcrumb, preview, blob, dim) = r;
            if blob.len() == dim * 4 {
                let vec_floats: &[f32] = unsafe {
                    std::slice::from_raw_parts(blob.as_ptr() as *const f32, dim)
                };
                let dot: f32 = q_vec.iter().zip(vec_floats).map(|(a, b)| a * b).sum();
                hits.push(SearchHit {
                    rel_path: rel,
                    stem,
                    domain,
                    title,
                    summary,
                    score: (dot * 10000.0).round() as f64 / 10000.0,
                    snippet: preview,
                    breadcrumb,
                    graph: None,
                });
            }
        }

        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        Ok(hits)
    }

    #[cfg(not(feature = "vectors"))]
    {
        let _ = (vault, query, domain_filter, limit);
        Ok(Vec::new())
    }
}
