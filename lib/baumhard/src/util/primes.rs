use lazy_static::lazy_static;

pub const PRIME_CEILING: usize = 10_000;

// Function to mark multiples of a prime as non-prime
fn mark_non_primes(sieve: &mut [bool], p: usize, max: usize) {
   let mut multiple = p * p;
   while multiple <= max {
      sieve[multiple] = false;
      multiple += p;
   }
}

// Function to compute primes using Sieve of Eratosthenes
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

// Use lazy_static to compute primes once and store them as usize
lazy_static! {
    static ref PRIMES: Vec<usize> = sieve_of_eratosthenes(PRIME_CEILING);
}

// Function to check if a number is prime using the precomputed primes
pub fn is_prime(n: usize) -> bool {
   PRIMES.binary_search(&n).is_ok()
}

pub fn get_primes() -> Vec<usize> {
   PRIMES.to_vec()
}