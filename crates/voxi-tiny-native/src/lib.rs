use candle_core::{Device, Tensor};
use candle_transformers::models::quantized_t5::{Config, T5ForConditionalGeneration, VarBuilder};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, time::Instant};
use tokenizers::Tokenizer;

pub const MAX_INPUT_TOKENS: usize = 64;
pub const MAX_NEW_TOKENS: usize = 64;
pub const MODEL_PACKAGE_ID: &str = "voxi-tiny-v3-q8_0-2026-11-16";
const CONFIG_SHA256: &str = "d833cc011ea12e96ff11e8c718223206a7a8985f35e553684602bcfb432aede9";
const TOKENIZER_SHA256: &str = "4629b0bb9b2c52f0a1364a8db3f609697e2ca4179e3d5ad3470a3267b4aeedd6";
const WEIGHTS_SHA256: &str = "10ee3c175bf71a77f9104cb6293416721bc81a174b97a1e6dc52715543986245";
pub const PLAN_PACKAGE_ID: &str = "voxi-plan-v4c-q8_0-2026-11-17";
const PLAN_TOKENIZER_SHA256: &str = "c9dda81803280849da5f5c63f9c6781cebd0c866601099fa201d28f5f68f6db8";
const PLAN_WEIGHTS_SHA256: &str = "7f0c2139e54d682d9acb7bf477ec938975f5f169a049af633ea81beea17deb99";

#[derive(Debug)]
pub enum NativeError {
    InvalidInput(String),
    InputTooLong { actual: usize, limit: usize },
    NoEos,
    PackageMismatch { file: String },
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
            Self::PackageMismatch { file } => write!(
                f,
                "package_mismatch: {file} is not part of the pinned model package"
            ),
            Self::Model(s) => write!(f, "model_error: {s}"),
        }
    }
}

impl std::error::Error for NativeError {}

#[derive(Deserialize)]
struct ConfigIds {
    eos_token_id: usize,
    pad_token_id: usize,
    decoder_start_token_id: usize,
}

pub struct NativeRewriter {
    pool: rayon::ThreadPool,
    core: RewriterCore,
}

struct RewriterCore {
    model: T5ForConditionalGeneration,
    tokenizer: Tokenizer,
    device: Device,
    eos_id: usize,
    start_id: usize,
    max_input: usize,
    max_output: usize,
    unknown_id: Option<u32>,
}

impl NativeRewriter {
    pub fn load(model_dir: impl AsRef<Path>) -> Result<Self, NativeError> {
        Self::load_with_threads(model_dir, 1)
    }

    /// Load with a dedicated CPU pool, independent of the host's Rayon pool.
    /// Instances and failed/retried loads never reconfigure process-global state.
    pub fn load_with_threads(
        model_dir: impl AsRef<Path>,
        threads: usize,
    ) -> Result<Self, NativeError> {
        Self::load_package(model_dir, threads, false)
    }

    /// Separately pinned structured-plan candidate. V3 remains unchanged.
    pub fn load_plan(model_dir: impl AsRef<Path>) -> Result<Self, NativeError> {
        Self::load_package(model_dir, 1, true)
    }

    fn load_package(model_dir: impl AsRef<Path>, threads: usize, plan: bool) -> Result<Self, NativeError> {
        if threads == 0 || threads > 64 {
            return Err(NativeError::InvalidInput(
                "threads must be between 1 and 64".into(),
            ));
        }
        let dir = model_dir.as_ref();
        let config_path = dir.join("config.json");
        let model_path = dir.join("model-q8_0.gguf");
        let tokenizer_path = dir.join("tokenizer.json");
        for p in [&config_path, &model_path, &tokenizer_path] {
            if !p.is_file() {
                return Err(NativeError::Model(format!("missing {}", p.display())));
            }
        }
        let cfg_bytes = verified_bytes(&config_path, CONFIG_SHA256)?;
        let tokenizer_bytes = verified_bytes(&tokenizer_path, if plan { PLAN_TOKENIZER_SHA256 } else { TOKENIZER_SHA256 })?;
        let model_bytes = verified_bytes(&model_path, if plan { PLAN_WEIGHTS_SHA256 } else { WEIGHTS_SHA256 })?;
        let ids: ConfigIds = serde_json::from_slice(&cfg_bytes).map_err(model_err)?;
        let mut model_cfg: serde_json::Value =
            serde_json::from_slice(&cfg_bytes).map_err(model_err)?;
        // The Candle decoder cache is designed for one-token incremental decode;
        // our bounded implementation replays the decoder prefix on each step.
        model_cfg["use_cache"] = serde_json::Value::Bool(false);
        let cfg: Config = serde_json::from_value(model_cfg).map_err(model_err)?;
        let mut tokenizer = Tokenizer::from_bytes(&tokenizer_bytes)
            .map_err(|e| NativeError::Model(e.to_string()))?;
        // This checkpoint's tokenizer.json persists truncation at 64. Disable it
        // so length validation sees the complete token sequence and refuses it.
        tokenizer
            .with_truncation(None)
            .map_err(|e| NativeError::Model(e.to_string()))?;
        let device = Device::Cpu;
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .map_err(|e| NativeError::Model(format!("cannot configure CPU threads: {e}")))?;
        let model = pool.install(|| {
            let vb = VarBuilder::from_gguf_buffer(&model_bytes, &device).map_err(model_err)?;
            T5ForConditionalGeneration::load(vb, &cfg).map_err(model_err)
        })?;
        let _ = ids.pad_token_id;
        let unknown_id = if plan { tokenizer.token_to_id("<unk>") } else { None };
        Ok(Self {
            pool,
            core: RewriterCore {
                model,
                tokenizer,
                device,
                eos_id: ids.eos_token_id,
                start_id: ids.decoder_start_token_id,
                max_input: if plan { 192 } else { MAX_INPUT_TOKENS },
                max_output: if plan { 256 } else { MAX_NEW_TOKENS },
                unknown_id,
            },
        })
    }

    /// Deterministic CPU greedy rewrite. Errors are refusals; no truncation occurs.
    pub fn rewrite(&mut self, text: &str) -> Result<String, NativeError> {
        let core = &mut self.core;
        self.pool.install(|| core.rewrite(text))
    }
}

impl RewriterCore {
    fn rewrite(&mut self, text: &str) -> Result<String, NativeError> {
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
        if self.unknown_id.is_some_and(|id| input_ids.contains(&id)) {
            return Err(NativeError::InvalidInput("unknown characters; quote literal names so they can be preserved exactly".into()));
        }
        if input_ids.len() > self.max_input {
            return Err(NativeError::InputTooLong {
                actual: input_ids.len(),
                limit: self.max_input,
            });
        }
        let input: Vec<u32> = input_ids.to_vec();
        let input = Tensor::new(input, &self.device)
            .map_err(model_err)?
            .unsqueeze(0)
            .map_err(model_err)?;
        let mut decoder_ids = vec![self.start_id as u32];
        let encoder = self.model.encode(&input).map_err(model_err)?;
        for _ in 0..self.max_output {
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
            let next = greedy_token(&logits)?;
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

fn verify_identity(bytes: &[u8], expected: &str, file: &str) -> Result<(), NativeError> {
    if format!("{:x}", Sha256::digest(bytes)) != expected {
        return Err(NativeError::PackageMismatch {
            file: file.to_string(),
        });
    }
    Ok(())
}

fn verified_bytes(path: &Path, expected: &str) -> Result<Vec<u8>, NativeError> {
    let bytes = fs::read(path).map_err(model_err)?;
    verify_identity(&bytes, expected, &path.display().to_string())?;
    // The exact verified bytes are parsed, not a second read of a mutable path.
    Ok(bytes)
}

fn model_err(e: impl std::fmt::Display) -> NativeError {
    NativeError::Model(e.to_string())
}

// Match PyTorch argmax: equal maxima choose the first token ID.
fn greedy_token(logits: &[f32]) -> Result<u32, NativeError> {
    if logits.is_empty() || logits.iter().any(|value| !value.is_finite()) {
        return Err(NativeError::Model("empty or non-finite logits".into()));
    }
    let mut best = 0;
    for index in 1..logits.len() {
        if logits[index] > logits[best] {
            best = index;
        }
    }
    Ok(best as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_but_different_package_bytes_are_rejected() {
        let abc_sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify_identity(b"abc", abc_sha, "fixture").is_ok());
        assert!(matches!(
            verify_identity(b"abd", abc_sha, "fixture"),
            Err(NativeError::PackageMismatch { .. })
        ));
    }

    #[test]
    fn argmax_ties_match_pytorch_and_invalid_logits_refuse() {
        assert_eq!(greedy_token(&[0.0, 2.0, 2.0]).unwrap(), 1);
        assert!(greedy_token(&[]).is_err());
        assert!(greedy_token(&[f32::NAN, 1.0]).is_err());
    }

    #[test]
    fn failed_load_does_not_poison_later_load_attempts() {
        let missing =
            std::env::temp_dir().join(format!("voxi-missing-model-{}", std::process::id()));
        for _ in 0..2 {
            let error = NativeRewriter::load(&missing)
                .err()
                .expect("missing model must refuse");
            assert!(error.to_string().contains("missing"), "{error}");
        }
    }

    #[test]
    #[ignore = "requires VOXI_TINY_TEST_MODEL_DIR pointing to the exported local model"]
    fn instances_work_after_host_pool_initialization_and_failed_load() {
        rayon::join(|| 1, || 2); // Simulate a host that already uses Rayon.
        let model_dir = std::env::var("VOXI_TINY_TEST_MODEL_DIR").expect("export path");
        let missing =
            std::env::temp_dir().join(format!("voxi-missing-model-{}", std::process::id()));
        assert!(NativeRewriter::load(&missing).is_err());
        let mut first = NativeRewriter::load_with_threads(&model_dir, 1).unwrap();
        let mut second = NativeRewriter::load_with_threads(&model_dir, 2).unwrap();
        let text = "find files in Downloads";
        assert_eq!(first.rewrite(text).unwrap(), second.rewrite(text).unwrap());
    }
}
