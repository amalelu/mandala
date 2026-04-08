use crate::util::primes::is_prime;
use rustc_hash::FxHashMap;
use std::collections::BTreeSet;
use std::sync::{RwLock, RwLockReadGuard, TryLockError};
use thiserror::Error;

/// Scenes and Trees all have their own unique [RegionIndexer]
#[derive(Debug, Clone)]
pub struct RegionIndexer {
    // Region is accessed by Vec-index (all regions exists in the Vec), BTreeSet for the element-ID (GfxElement or Tree)
    index: Vec<BTreeSet<usize>>,
    reverse_index: FxHashMap<usize, BTreeSet<usize>>,
    use_reverse_index: bool,
}

impl RegionIndexer {
    pub fn new() -> Self {
        RegionIndexer {
            index: Vec::new(),
            reverse_index: Default::default(),
            use_reverse_index: true,
        }
    }

    pub fn new_without_reverse_index() -> Self {
        RegionIndexer {
            index: Vec::new(),
            reverse_index: Default::default(),
            use_reverse_index: false,
        }
    }

    pub fn initialize_with(&mut self, x: usize, y: usize) {
        self.initialize(x * y)
    }

    /// Initializes all regions
    pub fn initialize(&mut self, num_regions: usize) {
        if self.index.len() > 0 {
            self.index = Vec::new();
        }
        for _ in 0..num_regions {
            self.index.push(BTreeSet::new());
        }
    }

    /// Properly indexes (and reverse-indexes) the element_id with the region
    pub fn insert(&mut self, element_id: usize, region: usize) {
        self.index[region].insert(element_id);
        if self.use_reverse_index {
            if !self.reverse_index.contains_key(&element_id) {
                self.reverse_index.insert(element_id, BTreeSet::new());
            }
            self.reverse_index
                .get_mut(&element_id)
                .unwrap()
                .insert(region);
        }
    }

    /// Properly removes the element-region index / reverse-index
    pub fn remove(&mut self, element_id: usize, region: usize) {
        self.index[region].remove(&element_id);
        if self.use_reverse_index {
            if self.reverse_index.contains_key(&element_id) {
                self.reverse_index
                    .get_mut(&element_id)
                    .unwrap()
                    .remove(&region);
            }
        }
    }

    /// Returns a set of elements that occupies the region
    pub fn elements_in_region(&self, region: usize) -> BTreeSet<usize> {
        self.index[region].iter().cloned().collect()
    }

    pub fn index_as_ref(&self) -> &Vec<BTreeSet<usize>> {
        &self.index
    }

    pub fn reverse_index_as_ref(&self) -> &FxHashMap<usize, BTreeSet<usize>> {
        &self.reverse_index
    }

    /// The indexed version of [self.find_regions_for_element]
    pub fn get_reverse_index_for_element(&mut self, element_id: usize) -> BTreeSet<usize> {
        self.reverse_index
            .get(&element_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Iterates through all regions and returns a set of the ones containing element_id
    /// This should not be necessary to use with reverse_index enabled
    pub fn scan_regions_for_element(&mut self, element_id: usize) -> BTreeSet<usize> {
        let mut regions = BTreeSet::new();
        for region in self.index.iter() {
            if region.contains(&element_id) {
                regions.insert(element_id);
            }
        }
        regions
    }
}

impl Default for RegionIndexer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Error, Debug, PartialEq)]
pub enum RegionError {
    #[error("RegionParams are being updated")]
    Updating,
    #[error("The parameters used are invalid")]
    InvalidParameters(&'static str),
    #[error("A lock is poisoned")]
    Poisoned,
}

/// Shared between a Scene and its Trees
#[derive(Debug)]
pub struct RegionParams {
    /// The initial target value that the pixel-grid will be divided into horizontally and vertically
    /// For example a factor of 10 will attempt to create a 10x10 grid of regions
    /// But the actual factor used in practice may vary depending on the size of the window
    target_region_factor: RwLock<usize>,
    /// The current, effective region-factor being used for x / horizontal
    region_factor_x: RwLock<usize>,
    /// The current, effective region-factor being used for x / vertical
    region_factor_y: RwLock<usize>,
    /// The effective resolution, none of the dimensions should have a span equal to a prime number
    current_resolution: RwLock<(usize, usize)>,
    region_size_x: RwLock<usize>,
    region_size_y: RwLock<usize>,
}

impl RegionParams {
    pub fn new(target_region_factor: usize, resolution: (usize, usize)) -> Self {
        assert!(!is_prime(resolution.0));
        assert!(!is_prime(resolution.1));
        let region_factor_x =
            Self::calculate_actual_region_factor(target_region_factor, resolution.0);
        let region_factor_y =
            Self::calculate_actual_region_factor(target_region_factor, resolution.1);
        RegionParams {
            target_region_factor: RwLock::new(target_region_factor),
            region_factor_x: RwLock::new(region_factor_x),
            region_factor_y: RwLock::new(region_factor_y),
            current_resolution: RwLock::new(resolution),
            region_size_x: RwLock::new(resolution.0 / region_factor_x),
            region_size_y: RwLock::new(resolution.1 / region_factor_y),
        }
    }

    pub fn calculate_regions_intersected_by_rectangle(
        &self,
        start: (usize, usize),
        end: (usize, usize),
    ) -> Result<Vec<usize>, RegionError> {
        let current_resolution = self.read_current_resolution()?;

        if start.0 > end.0 || start.1 > end.1 {
            return Err(RegionError::InvalidParameters(
                "Start position is higher than end position",
            ));
        }

        if start.0 >= current_resolution.0 || start.1 >= current_resolution.1 {
            return Err(RegionError::InvalidParameters(
                "Start position is out of resolution bounds",
            ));
        }

        if end.0 >= current_resolution.0 || end.1 >= current_resolution.1 {
            return Err(RegionError::InvalidParameters(
                "End position is out of resolution bounds",
            ));
        }

        let mut output = Vec::new();
        let mut head = start;
        let total_regions = self.calc_num_regions()?;
        loop {
            let region = self.calculate_region_from_pixel(head)?;
            if head.0 > end.0 || head.1 > end.1 {
                if head.0 > end.0 && head.1 > end.1 {
                    break;
                }
                head = self.calculate_pixel_from_region(region + 1)?;
                continue;
            }
            output.push(region);

            if region + 1 >= total_regions {
                break;
            }
            head = self.calculate_pixel_from_region(region + 1)?;
        }
        Ok(output)
    }

    pub fn calculate_region_from_pixel(&self, pixel: (usize, usize)) -> Result<usize, RegionError> {
        let dimensions = self.read_current_resolution()?;
        if dimensions.0 <= pixel.0 || dimensions.1 <= pixel.1 {
            return Err(RegionError::InvalidParameters("Pixel is out of bounds"));
        }
        let region_x = self.read_region_size_x()?;
        let region_y = self.read_region_size_y()?;
        let region_factor_x = self.read_region_factor_x()?;

        Ok(pixel.1 / region_y * region_factor_x + (pixel.0 / region_x))
    }

    pub fn calculate_pixel_from_region(
        &self,
        region: usize,
    ) -> Result<(usize, usize), RegionError> {
        let num_regions = self.calc_num_regions()?;
        if region >= num_regions {
            return Err(RegionError::InvalidParameters("Region is out of bounds"));
        }
        let region_x = self.read_region_size_x()?;
        let region_y = self.read_region_size_y()?;
        let region_factor_x = self.read_region_factor_x()?;
        let pixel_x = (region % region_factor_x) * region_x;
        let pixel_y = (region / region_factor_x) * region_y;
        Ok((pixel_x, pixel_y))
    }

    pub fn calc_num_regions(&self) -> Result<usize, RegionError> {
        let region_factor_x = self.read_region_factor_x()?;
        let region_factor_y = self.read_region_factor_y()?;
        Ok(region_factor_x * region_factor_y)
    }

    pub fn read_region_size_y(&self) -> Result<usize, RegionError> {
        Self::read_locked_value(&self.region_size_y)
    }

    pub fn read_region_size_x(&self) -> Result<usize, RegionError> {
        Self::read_locked_value(&self.region_size_x)
    }

    pub fn read_current_resolution(&self) -> Result<(usize, usize), RegionError> {
        Self::read_locked_value(&self.current_resolution)
    }

    pub fn read_target_region_factor(&self) -> Result<usize, RegionError> {
        Self::read_locked_value(&self.target_region_factor)
    }

    pub fn read_region_factor_x(&self) -> Result<usize, RegionError> {
        Self::read_locked_value(&self.region_factor_x)
    }

    pub fn read_region_factor_y(&self) -> Result<usize, RegionError> {
        Self::read_locked_value(&self.region_factor_y)
    }

    fn read_locked_value<T: Copy>(lock: &RwLock<T>) -> Result<T, RegionError> {
        match lock.try_read() {
            Ok(value) => Ok(*value),
            Err(e) => match e {
                TryLockError::Poisoned(_) => Err(RegionError::Poisoned),
                TryLockError::WouldBlock => Err(RegionError::Updating),
            },
        }
    }

    pub fn adapt(&mut self, target_factor: usize, dimensions: (usize, usize)) {
        assert!(!is_prime(dimensions.0));
        assert!(!is_prime(dimensions.1));
        let new_x_factor = Self::calculate_actual_region_factor(target_factor, dimensions.0);
        let new_y_factor = Self::calculate_actual_region_factor(target_factor, dimensions.1);

        *self.current_resolution.write().unwrap() = dimensions;

        *self.target_region_factor.write().unwrap() = target_factor;

        *self.region_factor_x.write().unwrap() = new_x_factor;
        *self.region_factor_y.write().unwrap() = new_y_factor;

        *self.region_size_x.write().unwrap() = dimensions.0 / new_x_factor;
        *self.region_size_y.write().unwrap() = dimensions.1 / new_y_factor;
    }

    pub(crate) fn calculate_actual_region_factor(
        target_factor: usize,
        dimension_span: usize,
    ) -> usize {
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

#[derive(Debug, Clone, Copy)]
pub struct RegionElementKeyPair {
    region_num: usize,
    element_id: usize,
}

impl RegionElementKeyPair {
    pub fn new(region_num: usize, element_id: usize) -> Self {
        Self {
            region_num,
            element_id,
        }
    }

    pub fn element_id(&self) -> usize {
        self.element_id
    }

    pub fn region_num(&self) -> usize {
        self.region_num
    }
}
