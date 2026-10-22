// SPDX-License-Identifier: MPL-2.0

use crate::util::primes::{get_primes, is_prime, PRIME_CEILING};
use std::collections::HashSet;

#[test]
pub fn test_primes() {
    do_primes()
}

/// Cross-check `is_prime` against the cached sieve for every value
/// `0..=PRIME_CEILING`: every value that the sieve emits as prime
/// must report `true`, every other value must report `false`. Pins
/// the sieve / lookup contract on both sides (positive and
/// negative).
pub fn do_primes() {
    let primes_set: HashSet<usize> = get_primes().into_iter().collect();
    for n in 0..=PRIME_CEILING {
        assert_eq!(is_prime(n), primes_set.contains(&n), "is_prime mismatch for {n}");
    }
}

#[test]
pub fn test_is_prime_above_the_sieve_ceiling() {
    do_is_prime_above_the_sieve_ceiling()
}

/// Primality above [`PRIME_CEILING`] is answered, not declined.
/// Every case here returned `false` before the trial-division
/// fallback existed, so the three primes are the inputs that make
/// this test fail against the old body — and the composites are the
/// control, because a fallback that simply returned `true` above the
/// ceiling would satisfy the primes and nothing else.
///
/// The four bands are chosen for the branch each one lands in:
///
/// - just past the ceiling, where the sieve's own primes settle it;
/// - past `PRIME_CEILING²`, where √n outruns the sieve and the 6k±1
///   continuation has to run at all — `10_007 × 10_009` is the
///   smallest interesting shape, a semiprime both of whose factors
///   are above the ceiling, so *only* the continuation can find one;
/// - `2^31 − 1`, a Mersenne prime, where the continuation runs to
///   about 46,000 and must still come back `true`;
/// - and the sieved band itself, re-checked at the boundary so the
///   `n <= PRIME_CEILING` split cannot drop or double-count its
///   edge.
pub fn do_is_prime_above_the_sieve_ceiling() {
    assert!(is_prime(10_007), "the first prime above the ceiling");
    assert!(!is_prime(10_001), "10_001 = 73 x 137");

    assert!(
        !is_prime(100_160_063),
        "10_007 x 10_009, both factors above the ceiling"
    );
    assert!(is_prime(100_160_069), "the first prime past 10_007 x 10_009");

    assert!(is_prime(2_147_483_647), "2^31 - 1 is a Mersenne prime");
    assert!(!is_prime(2_147_483_649), "2^31 + 1 = 3 x 715_827_883");

    // The boundary itself stays with the sieve: 10_000 is composite,
    // and 9_973 — the largest prime below the ceiling — is prime.
    assert!(!is_prime(PRIME_CEILING));
    assert!(is_prime(9_973));
}
