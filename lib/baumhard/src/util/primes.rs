//! Precomputed primes below `PRIME_CEILING` via a lazy Sieve of
//! Eratosthenes. The sieve runs once on first access; subsequent
//! queries are a binary search into the cached vector.

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

   let mut primes = Vec::new();
   for i in 2..=max {
      if sieve[i] {
         primes.push(i);
      }
   }

   primes
}

lazy_static! {
    static ref PRIMES: Vec<usize> = sieve_of_eratosthenes(PRIME_CEILING);
}

/// Return `true` iff `n` is prime and `n <= PRIME_CEILING`. O(log n)
/// binary search into the cached sieve; first call forces the sieve
/// walk.
pub fn is_prime(n: usize) -> bool {
   PRIMES.binary_search(&n).is_ok()
}

/// Return a freshly-cloned `Vec<usize>` of all primes up to
/// [`PRIME_CEILING`]. Allocates; callers that only need containment
/// should prefer [`is_prime`].
pub fn get_primes() -> Vec<usize> {
   PRIMES.to_vec()
}