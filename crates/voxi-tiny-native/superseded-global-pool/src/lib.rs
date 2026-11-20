use candle_core::{Device, Tensor};
use candle_transformers::models::quantized_t5::{Config, T5ForConditionalGeneration, VarBuilder};
use serde::Deserialize;
use std::{fs, path::Path, time::Instant};
use tokenizers::Tokenizer;

pub const MAX_INPUT_TOKENS: usize = 64;
pub const MAX_NEW_TOKENS: usize = 64;

#[derive(Debug)]
pub enum NativeError {
    InvalidInput(String),
    InputTooLong { actual: usize, limit: usize },
    NoEos,
    Model(String),
}

impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(s) => write!(f, "invalid_input: {s}"),
            Self::InputTooLong { actual, limit } => {
                write!(f, "input_too_long: {actual} tokens exceeds {limit}")
            }
            Self::NoEos => write!(f, "max_new_tokens reached without eos"),
            Self::Model(s) => write!(f, "model_error: {s}"),
        }
    }
}

#[derive(Deserialize)]
struct ConfigIds {
    eos_token_id: usize,
    pad_token_id: usize,
    decoder_start_token_id: usize,
}

pub struct NativeRewriter {
    model: T5ForConditionalGeneration,
    tokenizer: Tokenizer,
    device: Device,
    eos_id: usize,
    start_id: usize,
}

impl NativeRewriter {
    pub fn load(model_dir: impl AsRef<Path>) -> Result<Self, NativeError> {
        Self::load_with_threads(model_dir, 1)
    }

    /// Initialize Candle's process-wide CPU pool and load the model.
    /// Call before other code initializes Rayon in the host process.
    pub fn load_with_threads(
        model_dir: impl AsRef<Path>,
        threads: usize,
    ) -> Result<Self, NativeError> {
        if threads == 0 || threads > 64 {
            return Err(NativeError::InvalidInput(
                "threads must be between 1 and 64".into(),
            ));
        }
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .map_err(|e| NativeError::Model(format!("cannot configure CPU threads: {e}")))?;
        let dir = model_dir.as_ref();
        let config_path = dir.join("config.json");
        let model_path = dir.join("model-q8_0.gguf");
        let tokenizer_path = dir.join("tokenizer.json");
        for p in [&config_path, &model_path, &tokenizer_path] {
            if !p.is_file() {
                return Err(NativeError::Model(format!("missing {}", p.display())));
            }
        }
        let cfg_text = fs::read_to_string(&config_path).map_err(model_err)?;
        let ids: ConfigIds = serde_json::from_str(&cfg_text).map_err(model_err)?;
        let mut model_cfg: serde_json::Value =
            serde_json::from_str(&cfg_text).map_err(model_err)?;
        // The Candle decoder cache is designed for one-token incremental decode;
        // our bounded implementation replays the decoder prefix on each step.
        model_cfg["use_cache"] = serde_json::Value::Bool(false);
        let cfg: Config = serde_json::from_value(model_cfg).map_err(model_err)?;
        let mut tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| NativeError::Model(e.to_string()))?;
        // This checkpoint's tokenizer.json persists truncation at 64. Disable it
        // so length validation sees the complete token sequence and refuses it.
        tokenizer
            .with_truncation(None)
            .map_err(|e| NativeError::Model(e.to_string()))?;
        let device = Device::Cpu;
        let vb = VarBuilder::from_gguf(&model_path, &device).map_err(model_err)?;
        let model = T5ForConditionalGeneration::load(vb, &cfg).map_err(model_err)?;
        let _ = ids.pad_token_id;
        Ok(Self {
            model,
            tokenizer,
            device,
            eos_id: ids.eos_token_id,
            start_id: ids.decoder_start_token_id,
        })
    }

    /// Deterministic CPU greedy rewrite. Errors are refusals; no truncation occurs.
    pub fn rewrite(&mut self, text: &str) -> Result<String, NativeError> {
        if text.trim().is_empty() {
            return Err(NativeError::InvalidInput("text is empty".into()));
        }
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| NativeError::InvalidInput(e.to_string()))?;
        let input_ids = encoding.get_ids();
        if input_ids.is_empty() {
            return Err(NativeError::InvalidInput(
                "tokenizer produced no tokens".into(),
            ));
        }
        if input_ids.len() > MAX_INPUT_TOKENS {
            return Err(NativeError::InputTooLong {
                actual: input_ids.len(),
                limit: MAX_INPUT_TOKENS,
            });
        }
        let input: Vec<u32> = input_ids.to_vec();
        let input = Tensor::new(input, &self.device)
            .map_err(model_err)?
            .unsqueeze(0)
            .map_err(model_err)?;
        let mut decoder_ids = vec![self.start_id as u32];
        let encoder = self.model.encode(&input).map_err(model_err)?;
        for _ in 0..MAX_NEW_TOKENS {
            let decoder = Tensor::new(decoder_ids.clone(), &self.device)
                .map_err(model_err)?
                .unsqueeze(0)
                .map_err(model_err)?;
            let logits = self.model.decode(&decoder, &encoder).map_err(model_err)?;
            let logits = logits
                .squeeze(0)
                .map_err(model_err)?
                .to_vec1::<f32>()
                .map_err(model_err)?;
            let next = logits
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i as u32)
                .ok_or_else(|| NativeError::Model("empty logits".into()))?;
            if next as usize == self.eos_id {
                let generated = &decoder_ids[1..];
                return self
                    .tokenizer
                    .decode(generated, true)
                    .map_err(|e| NativeError::Model(e.to_string()));
            }
            decoder_ids.push(next);
        }
        Err(NativeError::NoEos)
    }
}

pub fn timed_rewrite(
    rewriter: &mut NativeRewriter,
    text: &str,
) -> (Result<String, NativeError>, f64) {
    let t = Instant::now();
    let value = rewriter.rewrite(text);
    (value, t.elapsed().as_secs_f64() * 1000.0)
}

fn model_err(e: impl std::fmt::Display) -> NativeError {
    NativeError::Model(e.to_string())
}
