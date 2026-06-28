//! NNUE training loop — supervised learning from CSA game results.
#![allow(clippy::needless_range_loop)] // index-based loops match matrix layout; don't change
//!
//! # Architecture
//!   Input → FT (L1=256, per perspective) → ClippedReLU → L2 (32) → ClippedReLU → Out
//!
//! # Algorithm
//!   teacher = game_result_from_stm_perspective × 600.0
//!   loss    = (score − teacher)²   where score = output / 64.0
//!   gradients backpropagated through ClippedReLU layers
//!   weights updated with Adam
//!
//! FT weights are quantised to i16 at save time; L2/out stay f32.

use std::collections::HashMap;

use sekirei_core::{
    board::Board,
    color::Color,
    movegen::is_in_check,
    nnue::{INPUT, L1, L2, NnueWeights, feature_index, hand_feature_index},
    piece::PieceKind,
    search::{SearchConfig, Searcher},
    sfen::board_to_sfen,
    tt::Tt,
};

use crate::csa::CsaGame;

// ---- Training weight container ----

pub struct TrainWeights {
    ft: Vec<f32>,      // INPUT × L1  (row-major: index = feat*L1 + neuron)
    ft_bias: Vec<f32>, // L1
    l2: Vec<f32>,      // 2*L1 × L2  (row-major: index = input_j*L2 + output_o)
    l2_bias: Vec<f32>, // L2
    out: Vec<f32>,     // L2
    out_bias: f32,

    // Adam first/second moments
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

impl TrainWeights {
    /// Zero-initialised weights.
    pub fn new() -> Self {
        let ft_len = INPUT * L1;
        let l2_len = 2 * L1 * L2;
        let out_len = L2;
        TrainWeights {
            ft: vec![0.0; ft_len],
            // Non-zero bias ensures ClippedReLU inputs are > 0 so gradients flow
            ft_bias: vec![0.5; L1],
            l2: vec![0.0; l2_len],
            l2_bias: vec![0.0; L2],
            out: vec![0.0; out_len],
            out_bias: 0.0,

            ft_m: vec![0.0; ft_len],
            ft_v: vec![0.0; ft_len],
            bias_m: vec![0.0; L1],
            bias_v: vec![0.0; L1],
            l2_m: vec![0.0; l2_len],
            l2_v: vec![0.0; l2_len],
            l2bias_m: vec![0.0; L2],
            l2bias_v: vec![0.0; L2],
            out_m: vec![0.0; out_len],
            out_v: vec![0.0; out_len],
            obias_m: 0.0,
            obias_v: 0.0,
            step: 0,
        }
    }

    /// Quantise FT to i16; L2/out stay f32.  Returns an NnueWeights ready for inference.
    pub fn to_nnue_weights(&self) -> NnueWeights {
        // FT: f32 → i16, scaled by FT_SCALE so small weights (≈±0.1) survive quantisation.
        // Inference must divide by FT_SCALE after ClippedReLU to recover the float equivalent.
        const FT_SCALE: f32 = 64.0;
        let mut ft = vec![[0i16; L1]; INPUT];
        for i in 0..INPUT {
            for j in 0..L1 {
                ft[i][j] = (self.ft[i * L1 + j] * FT_SCALE).clamp(-32767.0, 32767.0) as i16;
            }
        }
        let mut ft_bias = [0i16; L1];
        for (i, &v) in self.ft_bias.iter().enumerate() {
            ft_bias[i] = (v * FT_SCALE).clamp(-32767.0, 32767.0) as i16;
        }

        // L2 / out: f32 → f32 (no quantisation)
        let mut l2 = vec![[0.0f32; L2]; 2 * L1];
        for i in 0..2 * L1 {
            for o in 0..L2 {
                l2[i][o] = self.l2[i * L2 + o];
            }
        }
        let mut l2_bias = [0.0f32; L2];
        l2_bias.copy_from_slice(&self.l2_bias);

        let mut out = [0.0f32; L2];
        out.copy_from_slice(&self.out);

        NnueWeights {
            ft,
            ft_bias,
            l2,
            l2_bias,
            out,
            out_bias: self.out_bias,
        }
    }
}

// ---- Trainer ----

pub struct Trainer {
    pub weights: TrainWeights,
    pub total_loss: f64,
    pub total_count: u64,
    pub lr: f32,
    searcher: Searcher,
}

impl Trainer {
    pub fn new() -> Self {
        let tt = Tt::new(4); // Tt::new returns Arc<Tt>
        Trainer {
            weights: TrainWeights::new(),
            total_loss: 0.0,
            total_count: 0,
            lr: 0.001,
            searcher: Searcher::new(tt),
        }
    }

    /// Train on a single game.  Samples every `sample_every` plies.
    #[allow(clippy::too_many_arguments)]
    pub fn train_game(
        &mut self,
        game: &CsaGame,
        sample_every: usize,
        quiet: bool,
        min_ply: usize,
        label_depth: u32,
        scored: &HashMap<String, f32>,
        stability_weighted: bool,
    ) {
        let mut board = Board::startpos();

        for (ply, &mv) in game.moves.iter().enumerate() {
            if ply < min_ply || ply % sample_every != 0 {
                board.do_move(mv);
                continue;
            }

            if quiet {
                // skip positions in check (tactically unstable)
                if is_in_check(&board, board.side_to_move) {
                    board.do_move(mv);
                    continue;
                }
                // skip if next move is a capture (tactically unstable)
                if board.piece_at(mv.to).is_some() {
                    board.do_move(mv);
                    continue;
                }
            }

            // quietset filter / weighting
            let weight = if scored.is_empty() {
                1.0f32
            } else {
                let sfen = board_to_sfen(&board);
                match scored.get(&sfen) {
                    Some(&s) => {
                        if stability_weighted {
                            s
                        } else {
                            1.0
                        }
                    }
                    None => {
                        board.do_move(mv);
                        continue; // not in keep set
                    }
                }
            };

            let config = SearchConfig {
                max_depth: label_depth,
                time_limit: None,
            };
            let info = self.searcher.search(&mut board, config);
            let teacher = (info.score as f32).clamp(-600.0, 600.0);
            self.train_position(&board, teacher, weight);

            board.do_move(mv);
        }
    }

    /// One SGD step on a single position. `weight` scales the loss (quietset stability).
    fn train_position(&mut self, board: &Board, teacher: f32, weight: f32) {
        let stm = board.side_to_move;
        let w = &self.weights;

        // ── Forward pass ──────────────────────────────────────────────────────

        // FT accumulation
        let mut acc_us = w.ft_bias.clone();
        let mut acc_them = acc_us.clone();

        let active_us = active_features(board, stm);
        let active_them = active_features(board, stm.flip());

        for feat in &active_us {
            let base = feat * L1;
            for j in 0..L1 {
                acc_us[j] += w.ft[base + j];
            }
        }
        for feat in &active_them {
            let base = feat * L1;
            for j in 0..L1 {
                acc_them[j] += w.ft[base + j];
            }
        }

        // FT ClippedReLU [0, 127]
        let relu_us: Vec<f32> = acc_us.iter().map(|&x| x.clamp(0.0, 127.0)).collect();
        let relu_them: Vec<f32> = acc_them.iter().map(|&x| x.clamp(0.0, 127.0)).collect();

        // L2 accumulation
        let mut l2_acc = w.l2_bias.clone(); // Vec<f32> len=L2
        for j in 0..L1 {
            let a = relu_us[j];
            let b = relu_them[j];
            let base_us = j * L2;
            let base_them = (L1 + j) * L2;
            for o in 0..L2 {
                l2_acc[o] += a * w.l2[base_us + o];
                l2_acc[o] += b * w.l2[base_them + o];
            }
        }

        // L2 ClippedReLU [0, 127]
        let relu_l2: Vec<f32> = l2_acc.iter().map(|&x| x.clamp(0.0, 127.0)).collect();

        // Output
        let mut output = w.out_bias;
        for o in 0..L2 {
            output += relu_l2[o] * w.out[o];
        }
        let score = output / 64.0;

        // ── Loss ──────────────────────────────────────────────────────────────

        let err = score - teacher;
        self.total_loss += (weight as f64) * (err * err) as f64;
        self.total_count += 1;

        // ── Backward pass ─────────────────────────────────────────────────────

        let d_score = weight * 2.0 * err;
        let d_output = d_score / 64.0;

        // Output layer gradients
        let mut d_out = vec![0.0f32; L2];
        for o in 0..L2 {
            d_out[o] = d_output * relu_l2[o];
        }
        let d_out_bias = d_output;

        // Backprop through L2 ClippedReLU
        let mut d_l2_acc = [0.0f32; L2];
        for o in 0..L2 {
            if l2_acc[o] > 0.0 && l2_acc[o] < 127.0 {
                d_l2_acc[o] = d_output * self.weights.out[o];
            }
        }

        // L2 weight gradients and propagate to FT
        let mut d_l2 = vec![0.0f32; 2 * L1 * L2];
        let mut d_l2_bias = vec![0.0f32; L2];
        let mut d_relu_us = vec![0.0f32; L1];
        let mut d_relu_them = vec![0.0f32; L1];

        for j in 0..L1 {
            let base_us = j * L2;
            let base_them = (L1 + j) * L2;
            for o in 0..L2 {
                let g = d_l2_acc[o];
                d_l2[base_us + o] += g * relu_us[j];
                d_l2[base_them + o] += g * relu_them[j];
                d_relu_us[j] += g * self.weights.l2[base_us + o];
                d_relu_them[j] += g * self.weights.l2[base_them + o];
            }
        }
        d_l2_bias[..L2].copy_from_slice(&d_l2_acc[..L2]);

        // Backprop through FT ClippedReLU
        let mut d_acc_us = vec![0.0f32; L1];
        let mut d_acc_them = vec![0.0f32; L1];
        for j in 0..L1 {
            if acc_us[j] > 0.0 && acc_us[j] < 127.0 {
                d_acc_us[j] = d_relu_us[j];
            }
            if acc_them[j] > 0.0 && acc_them[j] < 127.0 {
                d_acc_them[j] = d_relu_them[j];
            }
        }

        // FT weight gradients (sparse)
        let mut d_ft = vec![0.0f32; INPUT * L1];
        let mut d_bias = vec![0.0f32; L1];

        for feat in &active_us {
            let base = feat * L1;
            for j in 0..L1 {
                d_ft[base + j] += d_acc_us[j];
            }
        }
        for feat in &active_them {
            let base = feat * L1;
            for j in 0..L1 {
                d_ft[base + j] += d_acc_them[j];
            }
        }
        for j in 0..L1 {
            d_bias[j] = d_acc_us[j] + d_acc_them[j];
        }

        // ── Adam update ───────────────────────────────────────────────────────

        self.weights.step += 1;
        let t = self.weights.step;
        let lr = self.lr;

        adam_update_slice(
            &mut self.weights.ft,
            &mut self.weights.ft_m,
            &mut self.weights.ft_v,
            &d_ft,
            lr,
            t,
        );
        adam_update_slice(
            &mut self.weights.ft_bias,
            &mut self.weights.bias_m,
            &mut self.weights.bias_v,
            &d_bias,
            lr,
            t,
        );
        adam_update_slice(
            &mut self.weights.l2,
            &mut self.weights.l2_m,
            &mut self.weights.l2_v,
            &d_l2,
            lr,
            t,
        );
        adam_update_slice(
            &mut self.weights.l2_bias,
            &mut self.weights.l2bias_m,
            &mut self.weights.l2bias_v,
            &d_l2_bias,
            lr,
            t,
        );
        adam_update_slice(
            &mut self.weights.out,
            &mut self.weights.out_m,
            &mut self.weights.out_v,
            &d_out,
            lr,
            t,
        );
        adam_update_scalar(
            &mut self.weights.out_bias,
            &mut self.weights.obias_m,
            &mut self.weights.obias_v,
            d_out_bias,
            lr,
            t,
        );
    }

    pub fn avg_loss(&self) -> f64 {
        if self.total_count == 0 {
            return 0.0;
        }
        self.total_loss / self.total_count as f64
    }

    pub fn reset_stats(&mut self) {
        self.total_loss = 0.0;
        self.total_count = 0;
    }
}

// ---- Active feature extraction ----

fn active_features(board: &Board, perspective: Color) -> Vec<usize> {
    const ALL_KINDS: [PieceKind; 14] = [
        PieceKind::Fu,
        PieceKind::Kyou,
        PieceKind::Kei,
        PieceKind::Gin,
        PieceKind::Kin,
        PieceKind::Kaku,
        PieceKind::Hisha,
        PieceKind::Ou,
        PieceKind::Tokin,
        PieceKind::Narikyo,
        PieceKind::Narikei,
        PieceKind::Narigin,
        PieceKind::Uma,
        PieceKind::Ryu,
    ];
    const HAND_KINDS: [PieceKind; 7] = [
        PieceKind::Fu,
        PieceKind::Kyou,
        PieceKind::Kei,
        PieceKind::Gin,
        PieceKind::Kin,
        PieceKind::Kaku,
        PieceKind::Hisha,
    ];

    let mut features = Vec::with_capacity(60);
    // Board features
    for &kind in &ALL_KINDS {
        for color in [Color::Black, Color::White] {
            let mut bb = board.pieces(color, kind);
            while let Some(sq) = bb.pop_lsb() {
                features.push(feature_index(sq, kind, color, perspective));
            }
        }
    }
    // Hand features: "≥ N pieces of kind K in hand" threshold features
    for &kind in &HAND_KINDS {
        for color in [Color::Black, Color::White] {
            let count = board.hand(color).get(kind);
            for n in 1..=count {
                features.push(hand_feature_index(kind, n, color, perspective));
            }
        }
    }
    features
}

// ---- Adam helpers ----

fn adam_update_slice(
    params: &mut [f32],
    m: &mut [f32],
    v: &mut [f32],
    grads: &[f32],
    lr: f32,
    t: u64,
) {
    for i in 0..params.len() {
        adam_update_scalar(&mut params[i], &mut m[i], &mut v[i], grads[i], lr, t);
    }
}

#[inline]
fn adam_update_scalar(param: &mut f32, m: &mut f32, v: &mut f32, grad: f32, lr: f32, t: u64) {
    const B1: f32 = 0.9;
    const B2: f32 = 0.999;
    const EPS: f32 = 1e-8;

    *m = B1 * *m + (1.0 - B1) * grad;
    *v = B2 * *v + (1.0 - B2) * grad * grad;

    let m_hat = *m / (1.0 - B1.powi(t as i32));
    let v_hat = *v / (1.0 - B2.powi(t as i32));

    *param -= lr * m_hat / (v_hat.sqrt() + EPS);
}
