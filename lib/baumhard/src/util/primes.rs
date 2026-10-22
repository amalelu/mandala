// SPDX-License-Identifier: MPL-2.0

//! Primality. Primes below `PRIME_CEILING` come from a lazy Sieve of
//! Eratosthenes — the sieve runs once on first access and queries
//! are a binary search into the cached vector. Larger arguments are
//! decided by trial division against that same table, so the ceiling
//! is a performance boundary rather than a correctness one: nothing
//! above it is reported as composite for want of being sieved.

use lazy_static::lazy_static;

/// Upper bound (inclusive) of the precomputed sieve.
pub const PRIME_CEILING: usize = 10_000;

fn mark_non_primes(sieve: &mut [bool], p: usize, max: usize) {
    let mut multiple = p * p;
    while multiple <= max {
        sieve[multiple] = false;
        multiple += p;
    }
}

fn sieve_of_eratosthenes(max: usize) -> Vec<usize> {
    let mut sieve = vec![true; max + 1];
    sieve[0] = false;
    sieve[1] = false;

    let mut p = 2;
    while p * p <= max {
        if sieve[p] {
            mark_non_primes(&mut sieve, p, max);
        }
        p += 1;
    }

    sieve
        .iter()
        .enumerate()
        .skip(2)
        .filter_map(|(n, &prime)| prime.then_some(n))
        .collect()
}

lazy_static! {
    static ref PRIMES: Vec<usize> = sieve_of_eratosthenes(PRIME_CEILING);
}

/// Return `true` iff `n` is prime — for **every** `n`, not only the
/// sieved ones.
///
/// The sieve answers up to [`PRIME_CEILING`]; above it this used to
/// return `false` for every argument, which is not "unknown" but a
/// wrong answer, and the caller that matters treats it as an answer.
/// `RegionParams::new` asserts `!is_prime(resolution)` precisely to
/// keep a prime dimension out of the degenerate-grid case, so a
/// 10,007-pixel canvas walked straight through the guard built to
/// stop it.
///
/// **Cost.** `n <= PRIME_CEILING`: O(log n) binary search into the
/// cached sieve, with the first call anywhere forcing the sieve
/// walk. Above it: trial division, roughly O(√n / ln √n) modulo
/// operations, taking the sieve's primes first and continuing with
/// 6k±1 candidates once √n outruns them (which needs `n` past
/// `PRIME_CEILING²`, a hundred million). No heap either way.
pub fn is_prime(n: usize) -> bool {
    if n <= PRIME_CEILING {
        return PRIMES.binary_search(&n).is_ok();
    }
    for &p in PRIMES.iter() {
        if p > n / p {
            return true;
        }
        if n.is_multiple_of(p) {
            return false;
        }
    }
    // The sieve ran out before √n did. Every prime factor below the
    // ceiling has been ruled out, so the remaining candidates start
    // at the first 6k−1 above it. Composite candidates cost a
    // division and find nothing — their own prime factors are all
    // below the ceiling and already tested — which is cheaper than
    // sieving further at construction frequency.
    let mut d = 6 * (PRIME_CEILING / 6) + 5;
    while d <= n / d {
        if n.is_multiple_of(d) || n.is_multiple_of(d + 2) {
            return false;
        }
        d += 6;
    }
    true
}

/// Return a freshly-cloned `Vec<usize>` of all primes up to
/// [`PRIME_CEILING`]. Allocates; callers that only need containment
/// should prefer [`is_prime`].
pub fn get_primes() -> Vec<usize> {
    PRIMES.to_vec()
}
