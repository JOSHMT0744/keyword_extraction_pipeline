//! A deterministic pseudo-random source for corpus generation, built on `blake3`'s
//! extendable output (XOF) rather than pulling in `rand` — there is no `rand` anywhere in
//! this crate's dependency tree, and `blake3` is already a direct dependency for the
//! resource digests and `PipelineVersion`. `Hasher::finalize_xof()` gives an unbounded
//! deterministic keystream from a fixed seed, which is exactly what a reproducible corpus
//! generator needs and nothing more.

use blake3::{Hasher, OutputReader};

pub struct Rng {
    reader: OutputReader,
}

impl Rng {
    /// `domain` separates independent draws so that adding a new random choice in one
    /// place (say, which paragraph order a carrier uses) can never reshuffle an unrelated
    /// draw elsewhere (which plant fills which slot) just because they happen to run in a
    /// different order after the change. Each domain gets its own hash input, so its
    /// keystream is independent of every other domain's.
    pub fn new(seed: &str, domain: &str) -> Self {
        let mut h = Hasher::new();
        h.update(b"kep-injection-rng-v1\0");
        h.update(seed.as_bytes());
        h.update(&[0]);
        h.update(domain.as_bytes());
        Self { reader: h.finalize_xof() }
    }

    fn fill(&mut self, buf: &mut [u8]) {
        self.reader.fill(buf);
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut buf = [0u8; 8];
        self.fill(&mut buf);
        u64::from_le_bytes(buf)
    }

    /// A uniform value in `[0, n)`, by rejection sampling rather than `% n` — modulo
    /// introduces a small bias whenever `n` doesn't evenly divide 2^64, and rejection
    /// costs nothing here that a test suite need worry about.
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "below(0) has no valid output");
        if n == 1 {
            return 0;
        }
        let n64 = n as u64;
        let limit = u64::MAX - (u64::MAX % n64);
        loop {
            let v = self.next_u64();
            if v < limit {
                return (v % n64) as usize;
            }
        }
    }

    pub fn choose<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }

    /// Fisher–Yates, using `below()` for every swap index.
    pub fn shuffle<T>(&mut self, xs: &mut [T]) {
        for i in (1..xs.len()).rev() {
            let j = self.below(i + 1);
            xs.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_and_domain_produce_the_same_stream() {
        let mut a = Rng::new("seed-1", "domain-a");
        let mut b = Rng::new("seed-1", "domain-a");
        for _ in 0..20 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_domains_diverge_even_with_the_same_seed() {
        let mut a = Rng::new("seed-1", "domain-a");
        let mut b = Rng::new("seed-1", "domain-b");
        let seq_a: Vec<u64> = (0..10).map(|_| a.next_u64()).collect();
        let seq_b: Vec<u64> = (0..10).map(|_| b.next_u64()).collect();
        assert_ne!(seq_a, seq_b, "two domains under the same seed produced identical streams");
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new("seed-1", "domain-a");
        let mut b = Rng::new("seed-2", "domain-a");
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn below_never_reaches_the_bound() {
        let mut r = Rng::new("seed-below", "domain");
        for _ in 0..500 {
            assert!(r.below(7) < 7);
        }
    }

    #[test]
    fn below_one_is_always_zero() {
        let mut r = Rng::new("seed-below-one", "domain");
        for _ in 0..10 {
            assert_eq!(r.below(1), 0);
        }
    }

    #[test]
    fn shuffle_is_a_permutation_not_a_resample() {
        let mut r = Rng::new("seed-shuffle", "domain");
        let mut xs: Vec<u32> = (0..30).collect();
        r.shuffle(&mut xs);
        let mut sorted = xs.clone();
        sorted.sort();
        assert_eq!(sorted, (0..30).collect::<Vec<u32>>(), "shuffle must not drop or duplicate elements");
    }

    #[test]
    fn shuffle_is_deterministic_under_the_same_seed() {
        let mut xs_a: Vec<u32> = (0..20).collect();
        let mut xs_b: Vec<u32> = (0..20).collect();
        Rng::new("seed-x", "shuffle-domain").shuffle(&mut xs_a);
        Rng::new("seed-x", "shuffle-domain").shuffle(&mut xs_b);
        assert_eq!(xs_a, xs_b);
    }
}
