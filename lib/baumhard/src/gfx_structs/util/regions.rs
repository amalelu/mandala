// SPDX-License-Identifier: MPL-2.0

//! Grid-parameter management for the spatial region system.
//!
//! `RegionParams` computes how to subdivide a pixel resolution into
//! a 2-D grid of region buckets, avoiding prime-number dimensions
//! (which have no non-trivial divisors and therefore cannot be evenly
//! partitioned). The companion `RegionIndexer` (in
//! `super::region_indexer`, re-exported below) owns the index
//! structure itself.

use crate::util::primes::is_prime;
use std::sync::RwLock;

// Re-export so existing consumers that `use regions::RegionIndexer`
// continue to resolve.
pub use super::region_indexer::RegionIndexer;

/// Failure modes returned by [`RegionParams`] accessors and computation
/// methods.
///
/// Per `CODE_CONVENTIONS.md` §7, baumhard does not implement `Display` or
/// `std::error::Error` for its own enums — call sites match on the
/// variant directly. Variants:
///
/// - `InvalidParameters`: an input is out of range (pixel beyond the
///   resolution, region index past the live count, malformed rectangle).
///   The `&'static str` is a short, log-ready reason.
/// - `Poisoned`: the inner lock is poisoned — an earlier writer
///   panicked while holding it. The region state may be inconsistent;
///   the caller should not retry.
///
/// **Removed:** an earlier `Updating` variant signalled
/// "lock-would-block" from the per-field `try_read` shape. After the
/// 6-locks → 1-lock collapse (Batch 6 §6.4a), reads use a blocking
/// `RwLock::read` that always succeeds (or returns `Poisoned`), so
/// there is no longer a non-blocking failure mode to surface. Zero
/// external callers matched on `Updating` at the time of removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionError {
    /// One of the inputs (pixel, region index, rectangle bounds) is
    /// outside the live resolution / region grid. Carries a static
    /// reason for the log.
    InvalidParameters(&'static str),
    /// The inner lock is poisoned — an earlier write panicked while
    /// holding it. Region state may be inconsistent.
    Poisoned,
}

/// Snapshot of all six pixel-grid / region-bucket parameters,
/// guarded by a single [`RwLock`] in [`RegionParams`].
///
/// Lives in this module rather than as a public type because
/// callers don't need to construct an `Inner` directly —
/// [`RegionParams::new`] / [`RegionParams::adapt`] are the only
/// writers, and the read accessors return `Copy` values out of
/// the lock.
#[derive(Debug, Clone, Copy)]
struct Inner {
    /// Caller-requested subdivisions per axis. The effective factor
    /// (stored in `region_factor_x`/`_y`) is derived from this plus
    /// the current resolution — divisor-snapped to avoid fractional
    /// regions.
    target_region_factor: usize,
    /// Effective horizontal subdivisions in use after divisor-snap.
    region_factor_x: usize,
    /// Effective vertical subdivisions in use after divisor-snap.
    region_factor_y: usize,
    /// Canvas resolution the factors were snapped against. Neither
    /// dimension is prime (enforced by `new`/`adapt`).
    current_resolution: (usize, usize),
    /// Pixels per region along x (`resolution.0 / region_factor_x`).
    region_size_x: usize,
    /// Pixels per region along y (`resolution.1 / region_factor_y`).
    region_size_y: usize,
}

/// Shared pixel-grid / region-bucket parameters for one Scene and
/// its owned Trees. The six logical fields sit behind a single
/// [`RwLock`] — readers (hit-tests, renderer) take one read lock
/// per accessor call, the one writer ([`RegionParams::adapt`])
/// takes one write lock per `adapt` call. Atomic-as-a-group is
/// the natural shape: every reader needs the post-`adapt` state
/// or pre-`adapt` state, never a mix.
#[derive(Debug)]
pub struct RegionParams {
    inner: RwLock<Inner>,
}

impl RegionParams {
    /// Construct a [`RegionParams`] for the given target grid density
    /// and pixel resolution.
    ///
    /// # Grid adaptation algorithm
    ///
    /// The caller requests `target_region_factor` subdivisions per
    /// axis (e.g. 10 → aim for a 10x10 grid). Because region
    /// boundaries must land on exact pixel boundaries — no fractional
    /// regions — the constructor finds the **closest divisor** of each
    /// dimension to `target_region_factor` via
    /// `calculate_actual_region_factor`. This means the effective
    /// factor may differ from the target (and may differ between x
    /// and y when the dimensions are not equal). Region pixel sizes
    /// are then `resolution.N / effective_factor_N`.
    ///
    /// # Panics
    ///
    /// Asserts that neither dimension is prime — prime dimensions have
    /// only 1 and themselves as divisors, making fine-grained grids
    /// impossible. Callers round prime dimensions to the nearest
    /// composite before construction.
    ///
    /// # Costs
    ///
    /// O(sqrt(max(resolution.0, resolution.1))) for the divisor
    /// search, plus 6 `RwLock::new` calls. No heap beyond the locks.
    pub fn new(target_region_factor: usize, resolution: (usize, usize)) -> Self {
        assert!(!is_prime(resolution.0));
        assert!(!is_prime(resolution.1));
        let region_factor_x = Self::calculate_actual_region_factor(target_region_factor, resolution.0);
        let region_factor_y = Self::calculate_actual_region_factor(target_region_factor, resolution.1);
        RegionParams {
            inner: RwLock::new(Inner {
                target_region_factor,
                region_factor_x,
                region_factor_y,
                current_resolution: resolution,
                region_size_x: resolution.0 / region_factor_x,
                region_size_y: resolution.1 / region_factor_y,
            }),
        }
    }

    /// One read-lock acquisition; returns a copy of the inner
    /// snapshot (every field is `Copy`). Used by the per-accessor
    /// helpers — collapses the previous 4-sequential-locks
    /// per-call cost to one.
    fn read_inner(&self) -> Result<Inner, RegionError> {
        match self.inner.read() {
            Ok(g) => Ok(*g),
            Err(_) => Err(RegionError::Poisoned),
        }
    }

    /// Return every region bucket that a pixel-space rectangle
    /// overlaps.
    ///
    /// Both `start` and `end` are *inclusive* pixel coordinates.
    /// Regions are returned in row-major order (left-to-right,
    /// top-to-bottom).
    ///
    /// # Errors
    ///
    /// - `InvalidParameters` if `start` is component-wise greater
    ///   than `end`, or if either corner lies outside the current
    ///   resolution.
    ///
    /// # Costs
    ///
    /// O(output) — one push per intersected region, no redundant
    /// iteration. Two lock reads (`region_size`, `region_factor_x`)
    /// plus the validation reads.
    pub fn calculate_regions_intersected_by_rectangle(
        &self,
        start: (usize, usize),
        end: (usize, usize),
    ) -> Result<Vec<usize>, RegionError> {
        let inner = self.read_inner()?;

        if start.0 > end.0 || start.1 > end.1 {
            return Err(RegionError::InvalidParameters(
                "Start position is higher than end position",
            ));
        }

        if start.0 >= inner.current_resolution.0 || start.1 >= inner.current_resolution.1 {
            return Err(RegionError::InvalidParameters(
                "Start position is out of resolution bounds",
            ));
        }

        if end.0 >= inner.current_resolution.0 || end.1 >= inner.current_resolution.1 {
            return Err(RegionError::InvalidParameters(
                "End position is out of resolution bounds",
            ));
        }

        let col_start = start.0 / inner.region_size_x;
        let col_end = end.0 / inner.region_size_x;
        let row_start = start.1 / inner.region_size_y;
        let row_end = end.1 / inner.region_size_y;

        let mut output = Vec::with_capacity((col_end - col_start + 1) * (row_end - row_start + 1));
        for row in row_start..=row_end {
            for col in col_start..=col_end {
                output.push(row * inner.region_factor_x + col);
            }
        }
        Ok(output)
    }

    /// Translate a pixel coordinate into the index of the region
    /// bucket that contains it. Row-major ordering.
    ///
    /// # Errors
    /// - `InvalidParameters` if `pixel` lies at or beyond the
    ///   current resolution on either axis.
    ///
    /// # Costs
    /// O(1). Three lock reads (resolution, region sizes, factor_x).
    pub fn calculate_region_from_pixel(&self, pixel: (usize, usize)) -> Result<usize, RegionError> {
        let inner = self.read_inner()?;
        if inner.current_resolution.0 <= pixel.0 || inner.current_resolution.1 <= pixel.1 {
            return Err(RegionError::InvalidParameters("Pixel is out of bounds"));
        }
        Ok(pixel.1 / inner.region_size_y * inner.region_factor_x + (pixel.0 / inner.region_size_x))
    }

    /// Return the top-left pixel corner of the given region bucket.
    ///
    /// # Errors
    /// - `InvalidParameters` if `region` is past the live bucket count.
    ///
    /// # Costs
    /// O(1), four lock reads.
    pub fn calculate_pixel_from_region(&self, region: usize) -> Result<(usize, usize), RegionError> {
        let inner = self.read_inner()?;
        let num_regions = inner.region_factor_x * inner.region_factor_y;
        if region >= num_regions {
            return Err(RegionError::InvalidParameters("Region is out of bounds"));
        }
        let pixel_x = (region % inner.region_factor_x) * inner.region_size_x;
        let pixel_y = (region / inner.region_factor_x) * inner.region_size_y;
        Ok((pixel_x, pixel_y))
    }

    /// Effective `factor_x * factor_y` region bucket count. O(1),
    /// one lock read.
    pub fn calc_num_regions(&self) -> Result<usize, RegionError> {
        let inner = self.read_inner()?;
        Ok(inner.region_factor_x * inner.region_factor_y)
    }

    /// Read-lock accessor for the y-axis region size (pixels). O(1).
    pub fn read_region_size_y(&self) -> Result<usize, RegionError> {
        Ok(self.read_inner()?.region_size_y)
    }

    /// Read-lock accessor for the x-axis region size (pixels). O(1).
    pub fn read_region_size_x(&self) -> Result<usize, RegionError> {
        Ok(self.read_inner()?.region_size_x)
    }

    /// Read-lock accessor for the canvas resolution the factors are
    /// snapped against. O(1); see [`RegionError`] for the failure modes.
    pub fn read_current_resolution(&self) -> Result<(usize, usize), RegionError> {
        Ok(self.read_inner()?.current_resolution)
    }

    /// Read-lock accessor for the caller-requested target factor.
    /// Differs from the effective factors when the resolution forced
    /// a divisor snap. O(1).
    pub fn read_target_region_factor(&self) -> Result<usize, RegionError> {
        Ok(self.read_inner()?.target_region_factor)
    }

    /// Read-lock accessor for the effective x-axis factor. O(1).
    pub fn read_region_factor_x(&self) -> Result<usize, RegionError> {
        Ok(self.read_inner()?.region_factor_x)
    }

    /// Read-lock accessor for the effective y-axis factor. O(1).
    pub fn read_region_factor_y(&self) -> Result<usize, RegionError> {
        Ok(self.read_inner()?.region_factor_y)
    }

    /// Reconfigure for a new target factor and/or resolution. One
    /// write lock acquired; downstream readers either observe the
    /// pre-`adapt` snapshot or the post-`adapt` snapshot — never a
    /// torn mix.
    ///
    /// # Panics
    /// Asserts neither dimension is prime (same invariant as `new`).
    ///
    /// # Costs
    /// O(sqrt(max(dimensions.0, dimensions.1))) for the divisor
    /// search, plus one write-lock acquisition.
    pub fn adapt(&mut self, target_factor: usize, dimensions: (usize, usize)) {
        assert!(!is_prime(dimensions.0));
        assert!(!is_prime(dimensions.1));
        let new_x_factor = Self::calculate_actual_region_factor(target_factor, dimensions.0);
        let new_y_factor = Self::calculate_actual_region_factor(target_factor, dimensions.1);
        *self.inner.write().unwrap() = Inner {
            target_region_factor: target_factor,
            region_factor_x: new_x_factor,
            region_factor_y: new_y_factor,
            current_resolution: dimensions,
            region_size_x: dimensions.0 / new_x_factor,
            region_size_y: dimensions.1 / new_y_factor,
        };
    }

    pub(crate) fn calculate_actual_region_factor(target_factor: usize, dimension_span: usize) -> usize {
        if target_factor == 0 || dimension_span == 0 {
            return 1;
        }

        // If target_factor is a divisor of dimension_span, return it
        if dimension_span % target_factor == 0 {
            return target_factor;
        }

        if target_factor > dimension_span {
            // The target factor is higher than the amount of pixels that the dimension spans
            // So here we are returning a region factor that creates one region per pixel
            return dimension_span;
        }

        let mut closest_factor = 1;
        let mut min_diff = usize::MAX;

        let mut divisor = 1;
        while divisor * divisor <= dimension_span {
            if dimension_span % divisor == 0 {
                let corresponding_divisor = dimension_span / divisor;

                // Check the difference for the current divisor
                let diff = divisor.abs_diff(target_factor);
                if diff < min_diff || (diff == min_diff && divisor < closest_factor) {
                    min_diff = diff;
                    closest_factor = divisor;
                }

                // Check the difference for the corresponding divisor
                if divisor != corresponding_divisor {
                    let corr_diff = corresponding_divisor.abs_diff(target_factor);
                    if corr_diff < min_diff
                        || (corr_diff == min_diff && corresponding_divisor < closest_factor)
                    {
                        min_diff = corr_diff;
                        closest_factor = corresponding_divisor;
                    }
                }
            }
            divisor += 1;
        }
        closest_factor
    }
}

/// Pairing of a region bucket number and an element's unique id.
///
/// Reserved token for the future per-tree region-index seam: when the
/// region indexer is wired into `MutatorTree::apply_to`, a [`Tree`] will
/// emit these pairs so a scene-level consumer can keep a
/// `region → elements` map in step with the tree. Not currently sent
/// anywhere (CONVENTIONS.md §B6).
///
/// Copy + 16 bytes so shipping one through a channel will be free —
/// no heap, no clone.
///
/// [`Tree`]: crate::gfx_structs::tree::Tree
#[derive(Debug, Clone, Copy)]
pub struct RegionElementKeyPair {
    region_num: usize,
    element_id: usize,
}

impl RegionElementKeyPair {
    /// Construct a pair. Both fields are plain `usize`s — no
    /// validation; the sender is trusted to have produced them from
    /// a live tree / region bucket. O(1), no allocation.
    pub fn new(region_num: usize, element_id: usize) -> Self {
        Self {
            region_num,
            element_id,
        }
    }

    /// Element `unique_id` the pair refers to — matches the id the
    /// element was registered with in its owning tree's arena.
    pub fn element_id(&self) -> usize {
        self.element_id
    }

    /// Region bucket number the element belongs to (or was moving
    /// into, on a move event).
    pub fn region_num(&self) -> usize {
        self.region_num
    }
}
