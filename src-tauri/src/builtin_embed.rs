//! 内置 BERT 句向量（Candle）：默认 `BAAI/bge-small-zh-v1.5`，Mean pooling + L2 归一化（迭代 6.2）。

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use std::fs;
use std::path::{Path, PathBuf};
use tokenizers::{Tokenizer, TruncationDirection, TruncationParams, TruncationStrategy};

/// 与侧车 `model_id`、缓存目录名一致
pub const DEFAULT_MODEL_ID: &str = "bge-small-zh-v1.5";

const REQUIRED_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

/// 模型目录内三件套齐全则视为就绪
pub fn is_model_ready(cache_dir: &Path, bundle_dir: &Path) -> bool {
    model_dir_ready(cache_dir) || model_dir_ready(bundle_dir)
}

/// 单目录是否含完整模型文件（供 bundle / 缓存路径检测）
pub fn model_dir_ready(dir: &Path) -> bool {
    REQUIRED_FILES.iter().all(|f| dir.join(f).is_file())
}

/// 将 bundle 内模型复制到运行时缓存（幂等覆盖）
pub fn ensure_model_cached(cache_dir: &Path, bundle_dir: &Path) -> Result<(), String> {
    if !model_dir_ready(bundle_dir) {
        return Err(format!(
            "Embedding model bundle is incomplete under {:?}",
            bundle_dir
        ));
    }
    fs::create_dir_all(cache_dir)
        .map_err(|e| format!("Failed to create model cache dir: {e}"))?;
    for name in REQUIRED_FILES {
        let from = bundle_dir.join(name);
        let to = cache_dir.join(name);
        fs::copy(&from, &to).map_err(|e| format!("Failed to copy {name}: {e}"))?;
    }
    Ok(())
}

/// 解析实际使用的模型目录：优先缓存，否则从 bundle 填充缓存
pub fn resolve_model_dir(cache_dir: &Path, bundle_dir: &Path) -> Result<PathBuf, String> {
    if model_dir_ready(cache_dir) {
        return Ok(cache_dir.to_path_buf());
    }
    ensure_model_cached(cache_dir, bundle_dir)?;
    Ok(cache_dir.to_path_buf())
}

pub struct BuiltinEmbedModel {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    pub dim: usize,
}

fn mean_pool_l2(
    token_embeddings: &Tensor,
    attention_mask: &Tensor,
) -> Result<Vec<f32>, String> {
    let dims = token_embeddings.dims();
    if dims.len() != 3 {
        return Err("expected rank-3 token embeddings".to_string());
    }
    let (_b, seq_len, _h) = (dims[0], dims[1], dims[2]);
    let mask = attention_mask
        .reshape((1, seq_len, 1))
        .map_err(|e| e.to_string())?
        .to_dtype(DType::F32)
        .map_err(|e| e.to_string())?
        .broadcast_as(token_embeddings.shape())
        .map_err(|e| e.to_string())?;
    let masked = (token_embeddings * &mask).map_err(|e| e.to_string())?;
    let summed = masked.sum(1).map_err(|e| e.to_string())?;
    let mask_sum = attention_mask
        .sum(1)
        .map_err(|e| e.to_string())?
        .to_dtype(DType::F32)
        .map_err(|e| e.to_string())?
        .clamp(1e-9f64, f64::MAX)
        .map_err(|e| e.to_string())?;
    let mean = summed
        .broadcast_div(&mask_sum.reshape((1, 1)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let norm = mean
        .sqr()
        .map_err(|e| e.to_string())?
        .sum_keepdim(1)
        .map_err(|e| e.to_string())?
        .sqrt()
        .map_err(|e| e.to_string())?
        .clamp(1e-12f64, f64::MAX)
        .map_err(|e| e.to_string())?;
    let out = mean
        .broadcast_div(&norm)
        .map_err(|e| e.to_string())?
        .squeeze(0)
        .map_err(|e| e.to_string())?
        .to_vec1::<f32>()
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// 从目录加载权重与 tokenizer（`model_dir` 含 config.json / tokenizer.json / model.safetensors）
pub fn load_model(model_dir: &Path) -> Result<BuiltinEmbedModel, String> {
    let config_path = model_dir.join("config.json");
    let tok_path = model_dir.join("tokenizer.json");
    let weights_path = model_dir.join("model.safetensors");
    let config_s = fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
    let config: BertConfig = serde_json::from_str(&config_s).map_err(|e| e.to_string())?;
    let dim = config.hidden_size;
    let mut tokenizer = Tokenizer::from_file(tok_path)
        .map_err(|e| format!("tokenizer load: {e}"))?;
    // 与 config.json 中 max_position_embeddings 对齐，避免 2048 字分块导致超长序列在 forward 失败
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: 512,
            stride: 0,
            strategy: TruncationStrategy::LongestFirst,
            direction: TruncationDirection::Right,
        }))
        .map_err(|e| format!("tokenizer set truncation: {e}"))?;
    let device = Device::Cpu;
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device) }
        .map_err(|e| format!("safetensors mmap: {e}"))?;
    let model = BertModel::load(vb.pp(""), &config)
        .or_else(|_| BertModel::load(vb, &config))
        .map_err(|e| format!("bert load: {e}"))?;
    Ok(BuiltinEmbedModel {
        model,
        tokenizer,
        device,
        dim,
    })
}

impl BuiltinEmbedModel {
    fn encode_one_inner(&self, text: &str) -> Result<Vec<f32>, String> {
        let enc = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| format!("tokenize: {e}"))?;
        let ids = enc.get_ids();
        if ids.is_empty() {
            return Err("empty tokenization".to_string());
        }
        let input_ids_vec: Vec<u32> = ids.iter().map(|&x| x as u32).collect();
        let attn_vec: Vec<f32> = enc
            .get_attention_mask()
            .iter()
            .map(|&x| if x > 0 { 1f32 } else { 0f32 })
            .collect();
        let input_ids = Tensor::new(input_ids_vec.as_slice(), &self.device)
            .map_err(|e| e.to_string())?
            .unsqueeze(0)
            .map_err(|e| e.to_string())?;
        let token_type_ids = input_ids.zeros_like().map_err(|e| e.to_string())?;
        let attention_mask_tensor = Tensor::new(attn_vec.as_slice(), &self.device)
            .map_err(|e| e.to_string())?
            .unsqueeze(0)
            .map_err(|e| e.to_string())?;
        let hidden = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask_tensor))
            .map_err(|e| format!("bert forward: {e}"))?;
        mean_pool_l2(&hidden, &attention_mask_tensor)
    }
}

pub fn encode_batch(model: &BuiltinEmbedModel, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
    let mut out = Vec::with_capacity(texts.len());
    for t in texts {
        let v = model.encode_one_inner(t)?;
        if v.len() != model.dim {
            return Err(format!("dim mismatch: got {} expected {}", v.len(), model.dim));
        }
        out.push(v);
    }
    Ok(out)
}

pub fn encode_single(model: &BuiltinEmbedModel, text: &str) -> Result<Vec<f32>, String> {
    encode_batch(model, &[text])?
        .into_iter()
        .next()
        .ok_or_else(|| "no embedding".to_string())
}
