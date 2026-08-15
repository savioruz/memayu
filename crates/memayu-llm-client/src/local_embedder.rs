//! Local, dependency-light embedding backend powered by [Candle].
//!
//! This module provides an [`EmbedderProvider`] that runs a small
//! sentence-transformers BERT model fully on-device via pure Rust bindings. No
//! API key is required and nothing is sent over the network (other than a
//! one-time model download on first use, cached under
//! [`memayu_config::model_dir`]).
//!
//! The default model is `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2`,
//! a multilingual checkpoint that covers Bahasa Indonesia and English well,
//! mirroring the gap memayu closes against competitors that only ship English
//! ONNX models.
//!
//! [Candle]: https://github.com/huggingface/candle

use async_trait::async_trait;
use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert;
use memayu_core::{EmbedError, EmbedderProvider};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Counter used to keep concurrent downloads from clobbering each other's temp
/// file while a model is being fetched into the cache for the first time.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Default multilingual model id used when none is configured.
pub const DEFAULT_MODEL_ID: &str = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2";

/// Default Hugging Face revision to pin the model to.
pub const DEFAULT_REVISION: &str = "main";

/// Truncate inputs to this many tokens to keep latency predictable.
const DEFAULT_MAX_LENGTH: usize = 256;

/// A fully-loaded model plus its tokenizer, memoized behind a `OnceLock`.
struct LoadedModel {
    model: bert::BertModel,
    tokenizer: tokenizers::Tokenizer,
    device: Device,
}

/// Local BERT embedder backed by Candle.
pub struct LocalEmbedder {
    model_id: String,
    revision: String,
    cache_dir: PathBuf,
    max_length: usize,
    loaded: Arc<Mutex<Option<Arc<LoadedModel>>>>,
}

impl LocalEmbedder {
    /// Create a local embedder for `model_id`, caching downloads under
    /// [`memayu_config::model_dir`].
    pub fn new(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            revision: DEFAULT_REVISION.to_string(),
            cache_dir: memayu_config::model_dir(),
            max_length: DEFAULT_MAX_LENGTH,
            loaded: Arc::new(Mutex::new(None)),
        }
    }
}

fn embed_err<E: std::fmt::Display>(e: E) -> EmbedError {
    EmbedError::Other(format!("local embedder: {e}"))
}

/// Download `filename` for `model_id`/`revision` into an HF-style local cache
/// under `cache_dir`, returning the local path. Returns the cached file early
/// if it already exists. Unlike `hf-hub`'s sync API, this follows Hugging Face's
/// relative redirects correctly (HF returns a relative `Location` header for
/// non-LFS files, which `hf-hub` 0.3.x passes to ureq un-resolved and crashes
/// on).
fn download_file(
    cache_dir: &Path,
    model_id: &str,
    revision: &str,
    filename: &str,
) -> Result<PathBuf, EmbedError> {
    let dest_dir = cache_dir
        .join(format!("models--{}", model_id.replace('/', "--")))
        .join("snapshots")
        .join(revision);
    std::fs::create_dir_all(&dest_dir).map_err(embed_err)?;
    let dest = dest_dir.join(filename);
    if dest.exists() {
        return Ok(dest);
    }

    let url = format!("https://huggingface.co/{model_id}/resolve/{revision}/{filename}");
    let resp = ureq::get(&url)
        .call()
        .map_err(|e| embed_err(format!("download {filename} failed: {e}")))?;

    // Write to a unique temp file first so concurrent downloads don't clobber
    // each other, and a partial download never leaves a corrupt cache entry.
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dest.with_extension(format!("part.{}.{seq}", std::process::id()));
    let mut out = std::fs::File::create(&tmp).map_err(embed_err)?;
    std::io::copy(&mut resp.into_reader(), &mut out).map_err(embed_err)?;
    out.sync_all().map_err(embed_err)?;

    // Another thread may have finished the same download while we were busy;
    // if so, drop our copy and reuse theirs.
    if dest.exists() {
        let _ = std::fs::remove_file(&tmp);
        return Ok(dest);
    }
    std::fs::rename(&tmp, &dest).map_err(embed_err)?;

    Ok(dest)
}

fn load_model(
    model_id: &str,
    revision: &str,
    cache_dir: &Path,
) -> Result<Arc<LoadedModel>, EmbedError> {
    std::fs::create_dir_all(cache_dir).map_err(embed_err)?;

    let config_path = download_file(cache_dir, model_id, revision, "config.json")?;
    let tokenizer_path = download_file(cache_dir, model_id, revision, "tokenizer.json")?;
    let weights_path = download_file(cache_dir, model_id, revision, "model.safetensors")?;

    let device = Device::Cpu;
    let config_bytes = std::fs::read(&config_path).map_err(embed_err)?;
    let config: bert::Config = serde_json::from_slice(&config_bytes).map_err(embed_err)?;

    let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path).map_err(embed_err)?;

    // SAFETY: the safetensors file is trusted (downloaded from the model's HF
    // repo and cached locally); mmap is only used as the backing store.
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights_path], bert::DTYPE, &device) }
        .map_err(embed_err)?;
    let model = bert::BertModel::load(vb, &config).map_err(embed_err)?;

    Ok(Arc::new(LoadedModel {
        model,
        tokenizer,
        device,
    }))
}

fn embed_blocking(
    loaded: &Mutex<Option<Arc<LoadedModel>>>,
    model_id: &str,
    revision: &str,
    cache_dir: &Path,
    max_length: usize,
    text: &str,
) -> Result<Vec<f32>, EmbedError> {
    let loaded = {
        let mut guard = loaded
            .lock()
            .map_err(|e| embed_err(format!("lock poisoned: {e}")))?;
        if guard.is_none() {
            *guard = Some(load_model(model_id, revision, cache_dir)?);
        }
        Arc::clone(guard.as_ref().unwrap())
    };

    let encoded = loaded
        .tokenizer
        .encode(text, true)
        .map_err(|e| embed_err(format!("tokenization failed: {e}")))?;

    let mut ids = encoded.get_ids().to_vec();
    if ids.len() > max_length {
        ids.truncate(max_length);
    }

    let device = &loaded.device;
    let input = Tensor::new(&ids[..], device)
        .map_err(embed_err)?
        .unsqueeze(0)
        .map_err(embed_err)?;
    let token_type_ids = input.zeros_like().map_err(embed_err)?;

    // Single, unpadded sequence: pass `None` for the attention mask so candle
    // treats every token as attended.
    let output = loaded
        .model
        .forward(&input, &token_type_ids, None)
        .map_err(embed_err)?; // [1, S, H]

    // Mean pooling over all tokens, then L2 normalization.
    let seq_len = output.dim(1).map_err(embed_err)? as f32;
    let div = Tensor::new(seq_len, device).map_err(embed_err)?;
    let sum = output.sum(1).map_err(embed_err)?; // [1, H]
    let pooled = sum.broadcast_div(&div).map_err(embed_err)?; // [1, H]
    let pooled = pooled.squeeze(0).map_err(embed_err)?; // [H]

    let norm = pooled
        .sqr()
        .map_err(embed_err)?
        .sum_all()
        .map_err(embed_err)?
        .sqrt()
        .map_err(embed_err)?;
    let normalized = pooled.broadcast_div(&norm).map_err(embed_err)?;

    normalized
        .to_vec1::<f32>()
        .map_err(|e| embed_err(format!("embedding conversion failed: {e}")))
}

#[async_trait]
impl EmbedderProvider for LocalEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let model_id = self.model_id.clone();
        let revision = self.revision.clone();
        let cache_dir = self.cache_dir.clone();
        let max_length = self.max_length;
        let text = text.to_string();
        let loaded = Arc::clone(&self.loaded);

        tokio::task::spawn_blocking(move || {
            embed_blocking(&loaded, &model_id, &revision, &cache_dir, max_length, &text)
        })
        .await
        .map_err(|e| EmbedError::Other(format!("local embedder task panicked: {e}")))?
    }
}
