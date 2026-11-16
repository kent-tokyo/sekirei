"""Freeze and build isolated H01 source variants; never edit shared core."""
from pathlib import Path
import difflib
import subprocess

from prepare_phased import ROOT, HERE, SOURCE, OLD

PLAIN = Path('/private/tmp/sekirei-issue291-h01')
COUNTED = Path('/private/tmp/sekirei-issue291-h01-counted')
PATCH = HERE / 'h01.patch'
PROOF = ROOT / 'docs/experiments/jsa_tsume_2026-11-16/proof_driver.rs'
D01_DRIVER = Path('/private/tmp/sekirei-issue291-d01/crates/sekirei-core/examples/jsa_search.rs')

HASHER = '''
// These tables live only for one bounded mate search. Zobrist keys are already
// dispersed; HashMap still compares full keys when hash buckets collide.
#[derive(Default)]
struct MateHasher(u64);

impl Hasher for MateHasher {
    #[inline]
    fn finish(&self) -> u64 {
        let mut x = self.0;
        x ^= x >> 30;
        x = x.wrapping_mul(0xbf58476d1ce4e5b9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte))
                .rotate_left(27)
                .wrapping_mul(0x9e3779b185ebca87);
        }
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0 ^ value)
            .rotate_left(27)
            .wrapping_mul(0x9e3779b185ebca87);
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }
}

type MateMap<K, V> = HashMap<K, V, BuildHasherDefault<MateHasher>>;
'''


def candidate(source):
    assert source.count('use std::collections::HashMap;') == 1
    source = source.replace(
        'use std::collections::HashMap;',
        'use std::collections::HashMap;\nuse std::hash::{BuildHasherDefault, Hasher};' + HASHER,
        1,
    )
    assert source.count('HashMap<') == 4  # Three fields plus the type alias.
    source = source.replace('table: HashMap<', 'table: MateMap<')
    source = source.replace('solved: HashMap<', 'solved: MateMap<')
    source = source.replace('terminals: HashMap<', 'terminals: MateMap<')
    assert source.count('HashMap::new()') == 15
    return source.replace('HashMap::new()', 'MateMap::default()')


def unpack(dest):
    assert not dest.exists(), dest
    dest.mkdir()
    archive = subprocess.check_output(
        ['git', 'archive', SOURCE, 'Cargo.toml', 'Cargo.lock', 'crates'], cwd=ROOT,
    )
    subprocess.run(['tar', '-xf', '-', '-C', str(dest)], input=archive, check=True)
    examples = dest / 'crates/sekirei-core/examples'
    examples.mkdir(exist_ok=True)
    return examples


def main():
    assert not PATCH.exists()
    assert D01_DRIVER.is_file()
    base = subprocess.check_output(
        ['git', 'show', SOURCE + ':crates/sekirei-core/src/mate.rs'], cwd=ROOT,
        text=True,
    )
    updated = candidate(base)
    PATCH.write_text(''.join(difflib.unified_diff(
        base.splitlines(True), updated.splitlines(True),
        fromfile='a/crates/sekirei-core/src/mate.rs',
        tofile='b/crates/sekirei-core/src/mate.rs',
    )))
    for dest in (PLAIN, COUNTED):
        examples = unpack(dest)
        if dest == COUNTED:
            subprocess.run(['git', 'apply', str(HERE/'d01-instrumentation.patch')], cwd=dest, check=True)
        # The H01 diff is applied after the counter patch; git apply relocates
        # non-overlapping hunks by their context.
        subprocess.run(['git', 'apply', '--check', str(PATCH)], cwd=dest, check=True)
        subprocess.run(['git', 'apply', str(PATCH)], cwd=dest, check=True)
        (examples / 'jsa_search.rs').write_bytes(
            D01_DRIVER.read_bytes() if dest == COUNTED else OLD.read_bytes()
        )
        if dest == PLAIN:
            (examples / 'jsa_proof.rs').write_bytes(PROOF.read_bytes())
    print(PLAIN, COUNTED)


if __name__ == '__main__':
    main()
