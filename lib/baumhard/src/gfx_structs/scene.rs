use std::sync::{Arc, Mutex};
use crossbeam_channel::{Receiver, Sender};
use slab::Slab;
use crate::core::primitives::{Applicable, Bounded, Positioned};
use crate::gfx_structs::util::regions::{RegionElementKeyPair, RegionIndexer, RegionParams};
use crate::gfx_structs::tree::{Tree};

/// A scene is the highest level structure in Baumhard, it contains and manages a set of trees
/// It is designed for concurrent access, locking on tree-level
/// It indexes trees based on their position in the scene

pub struct Scene<T: Positioned + Bounded + Clone, M: Applicable<T>> {
   trees: Slab<Arc<Mutex<Tree<T, M>>>>,
   resolution: (usize, usize),
   region_params: Arc<RegionParams>,
   region_index: RegionIndexer,
   self_sender: Sender<RegionElementKeyPair>,
   self_receiver: Receiver<RegionElementKeyPair>,
}

impl <T: Positioned + Bounded + Clone, M: Applicable<T>> Scene<T, M> {

}
