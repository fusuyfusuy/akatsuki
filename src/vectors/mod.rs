//! Vector substrate: chunking, blob packing, Candle inference, and semantic retrieval.
//!
//! The vector index is a projection of the same markdown source as the FTS index,
//! stored in the `note_vectors` table of `.akatsuki/cache.db`. Without the
//! `vectors` cargo feature (or without downloaded weights) every function here
//! degrades to an explicit note — never to a silent keyword-only answer.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::Value;

use crate::search::SearchHit;
use crate::storage::parse_frontmatter;

/// Section windows are packed at this many characters.
const MAX_CHUNK_CHARS: usize = 1200;
/// Characters of a chunk kept as its preview.
const PREVIEW_CHARS: usize = 280;

pub fn get_models_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine user home directory")?;
    let dir = home.join(".cache/akatsuki/models/intfloat_multilingual-e5-small");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn are_models_available() -> bool {
    match get_models_dir() {
        Ok(dir) => {
            dir.join("tokenizer.json").is_file()
                && dir.join("config.json").is_file()
                && dir.join("model.safetensors").is_file()
        }
        Err(_) => false,
    }
}

pub fn feature_enabled() -> bool {
    cfg!(feature = "vectors")
}

/// One embeddable window of a note.
#[derive(Debug, Clone)]
pub struct NoteChunk {
    pub chunk_id: String,
    pub rel_path: String,
    pub stem: String,
    pub domain: String,
    pub display_title: String,
    pub display_summary: String,
    pub tags: String,
    pub breadcrumb: String,
    pub chunk_index: usize,
    pub total_chunks: usize,
    pub preview: String,
    pub embed_text: String,
}

/// Splits a note into embedding chunks: one per heading section (levels 1-4),
/// windowed at [`MAX_CHUNK_CHARS`] on paragraph boundaries, each carrying a
/// document header and breadcrumb so a chunk is interpretable on its own.
pub fn chunk_note(rel_path: &str, content: &str) -> Result<Vec<NoteChunk>> {
    let (fm, body) = parse_frontmatter(content)?;
    Ok(chunk_parsed(rel_path, &fm, &body))
}

fn chunk_parsed(rel_path: &str, fm: &Value, body: &str) -> Vec<NoteChunk> {
    let stem = Path::new(rel_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_path)
        .to_string();
    let domain = rel_path.split('/').next().unwrap_or("").to_string();
    let display_title = fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&stem)
        .to_string();
    let display_summary = fm
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let tags = match fm.get("tags") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" "),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };

    let mut header = format!("[Document: {}]", display_title);
    if !display_summary.is_empty() {
        header.push_str(&format!("\n[Summary: {}]", display_summary));
    }
    if !tags.is_empty() {
        header.push_str(&format!("\n[Tags: {}]", tags));
    }

    let heading_re = Regex::new(r"(?m)^(#{1,4})[ \t]+(.+)$").expect("static heading regex");
    let headings: Vec<regex::Captures> = heading_re.captures_iter(body).collect();

    let mut sections: Vec<(String, String)> = Vec::new();
    if headings.is_empty() {
        sections.push((String::new(), body.trim().to_string()));
    } else {
        let first_start = headings[0].get(0).expect("capture 0").start();
        let preamble = body[..first_start].trim();
        if !preamble.is_empty() {
            sections.push(("Overview".to_string(), preamble.to_string()));
        }
        for (index, caps) in headings.iter().enumerate() {
            let whole = caps.get(0).expect("capture 0");
            let title = caps.get(2).expect("capture 2").as_str().trim().to_string();
            let end = headings
                .get(index + 1)
                .map(|next| next.get(0).expect("capture 0").start())
                .unwrap_or(body.len());
            sections.push((title, body[whole.end()..end].trim().to_string()));
        }
    }

    let mut windows: Vec<(String, String)> = Vec::new();
    for (breadcrumb, text) in sections {
        if text.chars().count() <= MAX_CHUNK_CHARS {
            windows.push((breadcrumb, text));
            continue;
        }
        let mut buffer: Vec<&str> = Vec::new();
        let mut buffer_chars = 0usize;
        for paragraph in text.split("\n\n") {
            let paragraph = paragraph.trim();
            if paragraph.is_empty() {
                continue;
            }
            let paragraph_chars = paragraph.chars().count();
            if buffer_chars + paragraph_chars > MAX_CHUNK_CHARS && !buffer.is_empty() {
                windows.push((breadcrumb.clone(), buffer.join("\n\n")));
                buffer.clear();
                buffer_chars = 0;
            }
            buffer.push(paragraph);
            buffer_chars += paragraph_chars + 2;
        }
        if !buffer.is_empty() {
            windows.push((breadcrumb, buffer.join("\n\n")));
        }
    }

    if windows.is_empty() {
        let fallback = if display_summary.is_empty() {
            display_title.clone()
        } else {
            display_summary.clone()
        };
        windows.push((String::new(), fallback));
    }

    let total_chunks = windows.len();
    windows
        .into_iter()
        .enumerate()
        .map(|(index, (breadcrumb, text))| {
            let mut parts = vec![header.clone()];
            if !breadcrumb.is_empty() {
                parts.push(format!("[Breadcrumb: {}]", breadcrumb));
            }
            if !text.is_empty() {
                parts.push(text.clone());
            }
            let preview = if text.is_empty() {
                display_summary.clone()
            } else {
                text.chars().take(PREVIEW_CHARS).collect()
            };

            NoteChunk {
                chunk_id: format!("{}:{}", rel_path, index),
                rel_path: rel_path.to_string(),
                stem: stem.clone(),
                domain: domain.clone(),
                display_title: display_title.clone(),
                display_summary: display_summary.clone(),
                tags: tags.clone(),
                breadcrumb,
                chunk_index: index,
                total_chunks,
                preview,
                embed_text: parts.join("\n").trim().to_string(),
            }
        })
        .collect()
}

/// Little-endian float32 blob. The cache is written and read only by this binary,
/// so the layout is stated rather than inherited from the host.
pub fn pack_vector(vector: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob
}

pub fn unpack_vector(blob: &[u8]) -> Vec<f32> {
    blob.as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Why semantic ranking cannot contribute right now, or `None` when it can.
pub fn degradation_note() -> Option<String> {
    if !feature_enabled() {
        return Some(
            "semantic ranking unavailable: binary built without the `vectors` feature; results are keyword (BM25) only"
                .to_string(),
        );
    }
    if !are_models_available() {
        return Some(
            "semantic ranking unavailable: model weights missing (run `akatsuki setup-models`); results are keyword (BM25) only"
                .to_string(),
        );
    }
    None
}

/// Full explanation for a hybrid/vector request, including an empty vector index.
pub fn hybrid_note(vault: &Path) -> Option<String> {
    if let Some(note) = degradation_note() {
        return Some(note);
    }
    let db_path = vault.join(".akatsuki/cache.db");
    if !db_path.is_file() {
        return Some("semantic ranking inactive: vector index not built yet".to_string());
    }
    match rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(con) => {
            let count: i64 = con
                .query_row("SELECT count(*) FROM note_vectors", [], |r| r.get(0))
                .unwrap_or(0);
            if count == 0 {
                Some(
                    "semantic ranking inactive: vector index is empty; run `akatsuki reconcile`"
                        .to_string(),
                )
            } else {
                None
            }
        }
        Err(_) => Some("semantic ranking inactive: vector index unreadable".to_string()),
    }
}

pub fn setup_models() -> Result<String> {
    #[cfg(feature = "vectors")]
    {
        let target_dir = get_models_dir()?;
        println!(
            "Fetching intfloat/multilingual-e5-small weights into: {}",
            target_dir.display()
        );

        let api = hf_hub::api::sync::ApiBuilder::new()
            .with_cache_dir(target_dir.clone())
            .build()?;
        let repo = api.model("intfloat/multilingual-e5-small".to_string());

        for file in ["tokenizer.json", "config.json", "model.safetensors"] {
            println!("  Downloading {}...", file);
            let path = repo.get(file)?;
            let dest = target_dir.join(file);
            if path != dest {
                std::fs::copy(&path, &dest)?;
            }
        }

        Ok(format!(
            "Successfully provisioned Candle model weights to {}",
            target_dir.display()
        ))
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
                bail!(
                    "Model weights not found at {}. Run 'akatsuki setup-models' first.",
                    weights_path.display()
                );
            }

            let device = Device::Cpu;
            let config_str = std::fs::read_to_string(&config_path)?;
            let config: Config = serde_json::from_str(&config_str)?;

            let tokenizer = Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

            let vb = unsafe {
                candle_nn::VarBuilder::from_mmaped_safetensors(
                    &[weights_path],
                    candle_core::DType::F32,
                    &device,
                )?
            };

            let model = BertModel::load(vb, &config)?;

            Ok(Self {
                model,
                tokenizer,
                device,
            })
        }

        pub fn encode_query(&self, text: &str) -> Result<Vec<f32>> {
            self.encode_single(&format!("query: {}", text.trim()))
        }

        pub fn encode_passage(&self, text: &str) -> Result<Vec<f32>> {
            let trimmed = text.trim();
            if let Some(rest) = trimmed.strip_prefix("passage: ") {
                return self.encode_single(rest);
            }
            self.encode_single(&format!("passage: {}", trimmed))
        }

        fn encode_single(&self, input: &str) -> Result<Vec<f32>> {
            let encoding = self
                .tokenizer
                .encode(input, true)
                .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

            // BERT position embeddings are strictly bounded to max_position_embeddings (512).
            const MAX_TOKENS: usize = 512;
            let ids = encoding.get_ids();
            let type_ids = encoding.get_type_ids();
            let mask = encoding.get_attention_mask();
            let len = ids.len().min(MAX_TOKENS);

            let token_tensor = Tensor::new(&ids[..len], &self.device)?.unsqueeze(0)?;
            let token_type_tensor = Tensor::new(&type_ids[..len], &self.device)?.unsqueeze(0)?;
            let mask_tensor = Tensor::new(&mask[..len], &self.device)?.unsqueeze(0)?;

            let output =
                self.model
                    .forward(&token_tensor, &token_type_tensor, Some(&mask_tensor))?;

            // Mean pooling over the attention mask.
            let mask_expanded = mask_tensor
                .unsqueeze(2)?
                .to_dtype(candle_core::DType::F32)?;
            let sum_embeddings = output.broadcast_mul(&mask_expanded)?.sum(1)?;
            let sum_mask = mask_expanded.sum(1)?.clamp(1e-9, f64::MAX)?;
            let mean_pooled = sum_embeddings.broadcast_div(&sum_mask)?;

            // L2 normalize so cosine similarity is a plain dot product.
            let norm = mean_pooled.sqr()?.sum_keepdim(1)?.sqrt()?;
            let normalized = mean_pooled.broadcast_div(&norm)?;

            Ok(normalized.squeeze(0)?.to_vec1()?)
        }
    }
}

/// Embeds and stores the chunks of every given note, replacing prior chunks.
///
/// Returns a human-readable note about what happened (or why nothing did) so the
/// reconcile report never claims work it did not perform.
pub fn sync_note_vectors(
    con: &mut rusqlite::Connection,
    notes: &[(String, String)],
) -> Result<String> {
    #[cfg(not(feature = "vectors"))]
    {
        let _ = con;
        Ok(format!(
            "disabled: this binary lacks the `vectors` feature ({} changed note(s) left unembedded)",
            notes.len()
        ))
    }

    #[cfg(feature = "vectors")]
    {
        if notes.is_empty() {
            return Ok("up to date".to_string());
        }
        if !are_models_available() {
            return Ok(format!(
                "skipped {} changed note(s): model weights missing (run `akatsuki setup-models`)",
                notes.len()
            ));
        }

        let embedder = candle_engine::CandleEmbedder::load()?;
        let mut embedded = 0usize;
        let tx = con.transaction()?;

        for (rel_path, content) in notes {
            let chunks = chunk_note(rel_path, content)?;
            tx.execute(
                "DELETE FROM note_vectors WHERE rel_path = ?1",
                rusqlite::params![rel_path],
            )?;

            for chunk in &chunks {
                let vector = embedder.encode_passage(&chunk.embed_text)?;
                if vector.len() != crate::constants::DEFAULT_EMBED_DIM {
                    bail!(
                        "Embedding dimension {} does not match the configured {} (wrong model weights?)",
                        vector.len(),
                        crate::constants::DEFAULT_EMBED_DIM
                    );
                }
                tx.execute(
                    "INSERT OR REPLACE INTO note_vectors \
                     (chunk_id, rel_path, stem, domain, display_title, display_summary, tags, breadcrumb, chunk_index, total_chunks, preview, vector_blob, dim) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    rusqlite::params![
                        chunk.chunk_id,
                        chunk.rel_path,
                        chunk.stem,
                        chunk.domain,
                        chunk.display_title,
                        chunk.display_summary,
                        chunk.tags,
                        chunk.breadcrumb,
                        chunk.chunk_index as i64,
                        chunk.total_chunks as i64,
                        chunk.preview,
                        pack_vector(&vector),
                        vector.len() as i64,
                    ],
                )?;
                embedded += 1;
            }
        }

        tx.commit()?;
        Ok(format!(
            "embedded {} chunk(s) across {} changed note(s)",
            embedded,
            notes.len()
        ))
    }
}

pub fn search_vectors(
    vault: &Path,
    query: &str,
    domain_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    #[cfg(not(feature = "vectors"))]
    {
        let _ = (vault, query, domain_filter, limit);
        bail!("Vector search requires a binary built with `--features vectors`.");
    }

    #[cfg(feature = "vectors")]
    {
        if !are_models_available() {
            bail!("Vector search requires model weights; run `akatsuki setup-models`.");
        }

        let db_path = vault.join(".akatsuki/cache.db");
        if !db_path.is_file() {
            bail!("Vector index missing; run `akatsuki reconcile` first.");
        }

        let embedder = candle_engine::CandleEmbedder::load()?;
        let query_vector = embedder.encode_query(query)?;

        let con = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;

        let sql = if domain_filter.is_some() {
            "SELECT rel_path, stem, domain, display_title, display_summary, breadcrumb, preview, vector_blob, dim \
             FROM note_vectors WHERE domain = ?1"
        } else {
            "SELECT rel_path, stem, domain, display_title, display_summary, breadcrumb, preview, vector_blob, dim \
             FROM note_vectors"
        };
        let mut stmt = con.prepare(sql)?;

        struct NoteVectorRow {
            rel_path: String,
            stem: String,
            domain: String,
            title: String,
            summary: String,
            breadcrumb: Option<String>,
            preview: String,
            blob: Vec<u8>,
            dim: i64,
        }

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<NoteVectorRow> {
            Ok(NoteVectorRow {
                rel_path: row.get(0)?,
                stem: row.get(1)?,
                domain: row.get(2)?,
                title: row.get(3)?,
                summary: row.get(4)?,
                breadcrumb: row.get(5)?,
                preview: row.get(6)?,
                blob: row.get(7)?,
                dim: row.get(8)?,
            })
        };

        let rows: Vec<_> = if let Some(domain) = domain_filter {
            stmt.query_map(rusqlite::params![domain], map_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map([], map_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };

        // One hit per note: the best-scoring chunk represents it.
        let mut best: std::collections::HashMap<String, SearchHit> =
            std::collections::HashMap::new();
        for row in rows {
            let rel_path = row.rel_path;
            let stem = row.stem;
            let domain = row.domain;
            let title = row.title;
            let summary = row.summary;
            let breadcrumb = row.breadcrumb;
            let preview = row.preview;
            let blob = row.blob;
            let dim = row.dim;
            if dim as usize * 4 != blob.len() {
                continue;
            }
            let vector = unpack_vector(&blob);
            if vector.len() != query_vector.len() {
                bail!(
                    "Vector for '{}' has {} dimensions but the model produces {}; re-run `akatsuki reconcile`",
                    rel_path,
                    vector.len(),
                    query_vector.len()
                );
            }
            let score: f32 = query_vector
                .iter()
                .zip(vector.iter())
                .map(|(a, b)| a * b)
                .sum();
            let rounded = (score * 10000.0).round() as f64 / 10000.0;

            let candidate = SearchHit {
                rel_path: rel_path.clone(),
                stem,
                domain,
                title,
                summary,
                score: rounded,
                snippet: preview,
                breadcrumb,
                graph: None,
            };

            match best.get(&rel_path) {
                Some(existing) if existing.score >= candidate.score => {}
                _ => {
                    best.insert(rel_path, candidate);
                }
            }
        }

        let mut hits: Vec<SearchHit> = best.into_values().collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.rel_path.cmp(&b.rel_path))
        });
        hits.truncate(limit);
        Ok(hits)
    }
}
