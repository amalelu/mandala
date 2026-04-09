// This module contains simd operations used anywhere in the application engine
// https://doc.rust-lang.org/core/arch/index.html
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::_mm256_set_ps;

fn foo() {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            return unsafe { foo_avx2() };
        }
    }

    // fallback implementation without using AVX2
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn foo_avx2() {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::_mm256_set_ps;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::_mm256_set_ps;
    let eight_floats = _mm256_set_ps(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0);
}
