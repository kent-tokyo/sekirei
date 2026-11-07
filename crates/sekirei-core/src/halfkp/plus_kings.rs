//! Opt-in dense format. It shares only the fixed-width integer head and
//! accumulator storage with HalfKP(Friend); legacy bytes remain unchanged.

use super::*;

/// Immutable feature-map identity attached to a loaded network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureSet {
    /// Legacy HalfKP(Friend) map, excluding both kings.
    HalfKpFriend,
    /// Original features followed by own/enemy king columns per own king.
    HalfKpPlusKingsV1,
}

/// Original feature IDs remain fixed; append 81 × (81 + 81) king columns.
pub const PLUS_KINGS_INPUT_FEATURES: usize = INPUT_FEATURES + 81 * 162;
// Version, feature-map, input, FT width, hidden widths, shift, FV policy.
const HEADER: [u32; 8] = [1, 1, PLUS_KINGS_INPUT_FEATURES as u32, 256, 32, 32, 6, 1];
/// Exact bytes in a dense `SEKHK001` image, including its 40-byte header.
pub const PLUS_KINGS_FILE_BYTES: usize = 40
    + HALF_DIMS * 2
    + PLUS_KINGS_INPUT_FEATURES * HALF_DIMS * 2
    + HIDDEN * 4
    + HIDDEN * 2 * HALF_DIMS
    + HIDDEN * 4
    + HIDDEN * HIDDEN
    + 4
    + HIDDEN;

pub(super) fn king_feature(king: Square, sq: Square, color: Color, perspective: Color) -> usize {
    INPUT_FEATURES
        + format_square(king, perspective) * 162
        + usize::from(color != perspective) * 81
        + format_square(sq, perspective)
}

fn checked_bias<const N: usize>(bias: &[i32; N], inputs: usize) -> io::Result<()> {
    // Full i8 range and clipped input range bound every partial dense/pair sum.
    let margin = (inputs * 128 * 127) as i32;
    if bias
        .iter()
        .any(|&b| b < i32::MIN + margin || b > i32::MAX - margin)
    {
        return Err(invalid("HalfKpPlusKingsV1 affine bias lacks i32 headroom"));
    }
    Ok(())
}

impl HalfKpNetwork {
    /// Feature-map identity, fixed for the lifetime of this network.
    pub const fn feature_set(&self) -> FeatureSet {
        self.feature_set
    }

    /// Parse only the distinct `SEKHK001` dense format. FV policy 1 retains
    /// the explicit runtime FV_SCALE range 1..=128 and default 16. The file
    /// does not silently change that process option.
    pub fn from_plus_kings_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() != PLUS_KINGS_FILE_BYTES {
            return Err(invalid("incorrect HalfKpPlusKingsV1 image length"));
        }
        let mut r = Reader { bytes, pos: 0 };
        if r.take(8)? != PLUS_KINGS_MAGIC {
            return Err(invalid("not a HalfKpPlusKingsV1 dense image"));
        }
        for expected in HEADER {
            if r.u32()? != expected {
                return Err(invalid("unsupported HalfKpPlusKingsV1 header field"));
            }
        }
        let ft_bias = r.i16s(HALF_DIMS)?.try_into().unwrap();
        let ft_weights = r
            .i16s(PLUS_KINGS_INPUT_FEATURES * HALF_DIMS)?
            .into_boxed_slice();
        let l1_bias = r.i32s::<HIDDEN>()?;
        checked_bias(&l1_bias, 2 * HALF_DIMS)?;
        let l1_weights = r.i8s(HIDDEN * 2 * HALF_DIMS)?.into_boxed_slice();
        let l2_bias = r.i32s::<HIDDEN>()?;
        checked_bias(&l2_bias, HIDDEN)?;
        let l2_weights = r.i8s(HIDDEN * HIDDEN)?.into_boxed_slice();
        let [out_bias] = r.i32s::<1>()?;
        checked_bias(&[out_bias], HIDDEN)?;
        let out_weights = r.i8s(HIDDEN)?.try_into().unwrap();
        debug_assert_eq!(r.pos, bytes.len());
        Ok(Self {
            architecture: "HalfKpPlusKingsV1 256x2-32-32".into(),
            feature_set: FeatureSet::HalfKpPlusKingsV1,
            ft_bias: Box::new(ft_bias),
            ft_weights,
            l1_bias,
            l1_pairs: pair_columns(&l1_weights),
            l1_rows: weight_rows(&l1_weights),
            l1_weights,
            l2_bias,
            l2_pairs: pair_columns(&l2_weights),
            l2_rows: weight_rows(&l2_weights),
            l2_weights,
            out_bias,
            out_weights,
        })
    }

    pub(super) fn plus_kings_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PLUS_KINGS_FILE_BYTES);
        out.extend_from_slice(PLUS_KINGS_MAGIC);
        for field in HEADER {
            out.extend_from_slice(&field.to_le_bytes());
        }
        for v in self.ft_bias.iter().chain(self.ft_weights.iter()) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for (bias, weights) in [
            (&self.l1_bias[..], &self.l1_weights[..]),
            (&self.l2_bias[..], &self.l2_weights[..]),
            (std::slice::from_ref(&self.out_bias), &self.out_weights[..]),
        ] {
            for b in bias {
                out.extend_from_slice(&b.to_le_bytes());
            }
            out.extend(weights.iter().map(|&w| w as u8));
        }
        out
    }

    /// Synthetic owned fixture only; this is not a trained candidate.
    /// Legacy feature rows and dense head match [`Self::random`] exactly.
    pub fn random_plus_kings(seed: u64) -> Self {
        let mut net = Self::random(seed);
        let mut weights = net.ft_weights.into_vec();
        weights.extend(
            (weights.len()..PLUS_KINGS_INPUT_FEATURES * HALF_DIMS)
                .map(|i| ((i as u64).wrapping_mul(17).wrapping_add(seed) % 49) as i16 - 24),
        );
        net.ft_weights = weights.into_boxed_slice();
        net.feature_set = FeatureSet::HalfKpPlusKingsV1;
        net.architecture = "HalfKpPlusKingsV1 256x2-32-32".into();
        net
    }
}

#[cfg(test)]
mod tests;
