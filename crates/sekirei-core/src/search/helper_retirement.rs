//! Opt-in retirement at completed exact iteration boundaries. The separate
//! sampled writer registry measures data supplied to other workers, rather
//! than foreign data consumed by the retiring worker.
use super::*;

pub(crate) struct HelperRetirement {
    enabled: bool,
    progress: Box<[AtomicU64]>,
    iterations: Box<[AtomicU64]>,
    retired: Box<[AtomicBool]>,
    pub(super) reuse: TtReuseStats,
}

#[derive(Clone)]
pub(crate) struct RetirementEndpoint {
    pub(super) channel: Arc<HelperRetirement>,
    pub(super) worker: usize,
}

impl HelperRetirement {
    pub(crate) fn new(workers: usize, enabled: bool) -> Self {
        Self {
            enabled,
            progress: (0..workers).map(|_| AtomicU64::new(0)).collect(),
            iterations: (0..workers).map(|_| AtomicU64::new(0)).collect(),
            retired: (0..workers).map(|_| AtomicBool::new(false)).collect(),
            reuse: TtReuseStats::new(),
        }
    }

    pub(crate) fn reset(&self) {
        for value in &self.progress {
            value.store(0, Ordering::Relaxed);
        }
        for value in &self.iterations {
            value.store(0, Ordering::Relaxed);
        }
        for value in &self.retired {
            value.store(false, Ordering::Relaxed);
        }
        self.reuse.reset();
    }

    pub(crate) fn contribution(&self, worker: usize) -> u64 {
        self.reuse
            .contributions
            .get(worker)
            .map_or(0, |v| v.load(Ordering::Relaxed))
    }

    pub(crate) fn snapshot(&self) -> Vec<(u64, bool, u64)> {
        self.iterations
            .iter()
            .enumerate()
            .map(|(i, count)| {
                (
                    count.load(Ordering::Relaxed),
                    self.retired[i].load(Ordering::Acquire),
                    self.contribution(i),
                )
            })
            .collect()
    }

    /// Publish one exact completed iteration, then decide whether starting
    /// another iteration is useful. This never requests an abort.
    pub(crate) fn completed(
        &self,
        worker: usize,
        depth: u32,
        mv: Move,
        stable: bool,
        recent_contributions: u64,
    ) -> bool {
        let packed =
            u64::from(pack_killer(mv)) | (u64::from(depth) << 19) | (u64::from(stable) << 51);
        self.progress[worker].store(packed, Ordering::Release);
        self.iterations[worker].fetch_add(1, Ordering::Relaxed);
        if !self.enabled
            || worker == 0
            || !stable
            || self.iterations[worker].load(Ordering::Relaxed) < 3
        {
            return false;
        }
        // A lack of observations is not evidence of low contribution.
        let total = self.reuse.foreign_hits.load(Ordering::Relaxed);
        if self
            .reuse
            .dropped_stores
            .load(Ordering::Relaxed)
            .saturating_mul(20)
            > self.reuse.sampled_stores.load(Ordering::Relaxed)
        {
            return false;
        }
        if total < 32
            || recent_contributions > 1
            || self.contribution(worker).saturating_mul(20) > total
        {
            return false;
        }
        let deepest = self
            .progress
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(i, _)| !self.retired[*i].load(Ordering::Acquire))
            .map(|(i, value)| (i, value.load(Ordering::Acquire)))
            .max_by_key(|(i, value)| (((value >> 19) & 0xffff_ffff), std::cmp::Reverse(*i)));
        let Some((leader, value)) = deepest else {
            return false;
        };
        let leader_depth = ((value >> 19) & 0xffff_ffff) as u32;
        let same_move = (value & RootHintChannel::MOVE_MASK) == u64::from(pack_killer(mv));
        let leader_stable = value >> 51 != 0;
        if leader == worker
            || leader_depth < depth.saturating_add(2)
            || !same_move
            || !leader_stable
        {
            return false;
        }
        self.retired[worker].store(true, Ordering::Release);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retirement_requires_progress_agreement_and_observed_low_supplied_work() {
        let channel = HelperRetirement::new(4, true);
        let mv = Move::drop(Square::from_index(1), PieceKind::Fu);
        let other = Move::drop(Square::from_index(2), PieceKind::Fu);
        channel.completed(1, 16, mv, true, 0);
        channel.completed(2, 12, mv, true, 0);
        channel.completed(2, 13, mv, true, 0);
        assert!(!channel.completed(2, 14, mv, true, 0)); // no observed reuse
        channel.reuse.foreign_hits.store(40, Ordering::Relaxed);
        assert!(!channel.completed(0, 10, mv, true, 0));
        assert!(!channel.completed(1, 16, mv, true, 0)); // deepest survives
        assert!(!channel.completed(2, 14, other, true, 0));
        assert!(!channel.completed(2, 14, mv, false, 0));
        assert!(!channel.completed(2, 15, mv, true, 0)); // lag only one
        assert!(!channel.completed(2, 14, mv, true, 2)); // useful recently
        channel.reuse.contributions[2].store(3, Ordering::Relaxed);
        assert!(!channel.completed(2, 14, mv, true, 0)); // useful in invocation
        channel.reuse.contributions[2].store(1, Ordering::Relaxed);
        assert!(channel.completed(2, 14, mv, true, 0));
        assert!(channel.snapshot()[2].1);
        assert!(!channel.snapshot()[0].1);
        assert!(!channel.snapshot()[1].1);
        channel.reset();
        assert_eq!(channel.snapshot(), vec![(0, false, 0); 4]);
    }
}
