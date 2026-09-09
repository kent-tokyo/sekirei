//! Training checkpoint persistence.
//!
//! Inference weights and optimizer state intentionally use separate formats.
//! This module owns the training-only JSON schemas and their validation so the
//! optimizer implementation in `trainer.rs` does not also have to own file
//! format and atomic-write details.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use sekirei_core::nnue::{INPUT, L1, L2};

use super::TrainWeights;

const ADAM_SCHEMA: &str = "sekirei.adam-checkpoint.v1";
const RESUME_SCHEMA: &str = "sekirei.resume-checkpoint.v1";

#[derive(Debug, Serialize, Deserialize)]
struct AdamCheckpoint {
    schema: String,
    version: u32,
    ft: Vec<f32>,
    ft_bias: Vec<f32>,
    l2: Vec<f32>,
    l2_bias: Vec<f32>,
    out: Vec<f32>,
    out_bias: f32,
    ft_m: Vec<f32>,
    ft_v: Vec<f32>,
    bias_m: Vec<f32>,
    bias_v: Vec<f32>,
    l2_m: Vec<f32>,
    l2_v: Vec<f32>,
    l2bias_m: Vec<f32>,
    l2bias_v: Vec<f32>,
    out_m: Vec<f32>,
    out_v: Vec<f32>,
    obias_m: f32,
    obias_v: f32,
    step: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResumeCheckpoint {
    schema: String,
    version: u32,
    epoch_completed: u64,
    next_game_index: u64,
    config_fingerprint: String,
    #[serde(default)]
    teacher_cache: HashMap<String, i32>,
    optimizer: AdamCheckpoint,
}

/// State restored at an epoch boundary. `next_game_index` is retained in the
/// schema even though the current writer emits zero (the safe boundary after
/// validation); this makes the cursor explicit and rejects future partial
/// checkpoints unless the caller handles that cursor deliberately.
pub struct ResumeState {
    pub weights: TrainWeights,
    pub epoch_completed: u64,
    pub next_game_index: u64,
    pub config_fingerprint: String,
    pub teacher_cache: HashMap<String, i32>,
}

impl TrainWeights {
    fn adam_checkpoint(&self) -> AdamCheckpoint {
        AdamCheckpoint {
            schema: ADAM_SCHEMA.to_string(),
            version: 1,
            ft: self.ft.clone(),
            ft_bias: self.ft_bias.clone(),
            l2: self.l2.clone(),
            l2_bias: self.l2_bias.clone(),
            out: self.out.clone(),
            out_bias: self.out_bias,
            ft_m: self.ft_m.clone(),
            ft_v: self.ft_v.clone(),
            bias_m: self.bias_m.clone(),
            bias_v: self.bias_v.clone(),
            l2_m: self.l2_m.clone(),
            l2_v: self.l2_v.clone(),
            l2bias_m: self.l2bias_m.clone(),
            l2bias_v: self.l2bias_v.clone(),
            out_m: self.out_m.clone(),
            out_v: self.out_v.clone(),
            obias_m: self.obias_m,
            obias_v: self.obias_v,
            step: self.step,
        }
    }

    fn from_adam_checkpoint(state: AdamCheckpoint) -> io::Result<Self> {
        if state.schema != ADAM_SCHEMA || state.version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported Adam checkpoint schema",
            ));
        }
        let expected = [
            ("ft", state.ft.len(), INPUT * L1),
            ("ft_bias", state.ft_bias.len(), L1),
            ("l2", state.l2.len(), 2 * L1 * L2),
            ("l2_bias", state.l2_bias.len(), L2),
            ("out", state.out.len(), L2),
            ("ft_m", state.ft_m.len(), INPUT * L1),
            ("ft_v", state.ft_v.len(), INPUT * L1),
            ("bias_m", state.bias_m.len(), L1),
            ("bias_v", state.bias_v.len(), L1),
            ("l2_m", state.l2_m.len(), 2 * L1 * L2),
            ("l2_v", state.l2_v.len(), 2 * L1 * L2),
            ("l2bias_m", state.l2bias_m.len(), L2),
            ("l2bias_v", state.l2bias_v.len(), L2),
            ("out_m", state.out_m.len(), L2),
            ("out_v", state.out_v.len(), L2),
        ];
        if let Some((name, actual, expected)) = expected.into_iter().find(|(_, a, e)| a != e) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name} length {actual}, expected {expected}"),
            ));
        }
        let finite = state
            .ft
            .iter()
            .chain(&state.ft_bias)
            .chain(&state.l2)
            .chain(&state.l2_bias)
            .chain(&state.out)
            .chain(&state.ft_m)
            .chain(&state.ft_v)
            .chain(&state.bias_m)
            .chain(&state.bias_v)
            .chain(&state.l2_m)
            .chain(&state.l2_v)
            .chain(&state.l2bias_m)
            .chain(&state.l2bias_v)
            .chain(&state.out_m)
            .chain(&state.out_v)
            .all(|v| v.is_finite())
            && state.out_bias.is_finite()
            && state.obias_m.is_finite()
            && state.obias_v.is_finite();
        if !finite {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Adam checkpoint contains non-finite values",
            ));
        }
        Ok(Self {
            ft: state.ft,
            ft_bias: state.ft_bias,
            l2: state.l2,
            l2_bias: state.l2_bias,
            out: state.out,
            out_bias: state.out_bias,
            ft_m: state.ft_m,
            ft_v: state.ft_v,
            bias_m: state.bias_m,
            bias_v: state.bias_v,
            l2_m: state.l2_m,
            l2_v: state.l2_v,
            l2bias_m: state.l2bias_m,
            l2bias_v: state.l2bias_v,
            out_m: state.out_m,
            out_v: state.out_v,
            obias_m: state.obias_m,
            obias_v: state.obias_v,
            step: state.step,
        })
    }

    /// Save raw f32 weights and all Adam moments for a training-only resume.
    pub fn save_adam_checkpoint(&self, path: &Path) -> io::Result<()> {
        let state = self.adam_checkpoint();
        let bytes = serde_json::to_vec_pretty(&state)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        atomic_write(path, "adam.json.tmp", &bytes)
    }

    /// Load a training-only checkpoint and reject schema, shape, or non-finite
    /// data errors before any state is used by the optimizer.
    pub fn load_adam_checkpoint(path: &Path) -> io::Result<Self> {
        let state: AdamCheckpoint = serde_json::from_slice(&fs::read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Self::from_adam_checkpoint(state)
    }

    /// Save the complete epoch-boundary resume state.
    pub fn save_resume_checkpoint_with_cache(
        &self,
        path: &Path,
        epoch_completed: u64,
        next_game_index: u64,
        config_fingerprint: &str,
        teacher_cache: &HashMap<String, i32>,
    ) -> io::Result<()> {
        let state = ResumeCheckpoint {
            schema: RESUME_SCHEMA.to_string(),
            version: 1,
            epoch_completed,
            next_game_index,
            config_fingerprint: config_fingerprint.to_string(),
            teacher_cache: teacher_cache.clone(),
            optimizer: self.adam_checkpoint(),
        };
        let bytes = serde_json::to_vec_pretty(&state)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        atomic_write(path, "resume.json.tmp", &bytes)
    }

    pub fn load_resume_checkpoint(path: &Path) -> io::Result<ResumeState> {
        let state: ResumeCheckpoint = serde_json::from_slice(&fs::read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if state.schema != RESUME_SCHEMA || state.version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported resume checkpoint schema",
            ));
        }
        if state.config_fingerprint.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "resume checkpoint has empty config fingerprint",
            ));
        }
        Ok(ResumeState {
            weights: Self::from_adam_checkpoint(state.optimizer)?,
            epoch_completed: state.epoch_completed,
            next_game_index: state.next_game_index,
            config_fingerprint: state.config_fingerprint,
            teacher_cache: state.teacher_cache,
        })
    }
}

fn atomic_write(path: &Path, temporary_extension: &str, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(temporary_extension);
    let result = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
