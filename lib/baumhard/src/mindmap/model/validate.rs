// SPDX-License-Identifier: MPL-2.0

//! Shared validation for `MindNode` / `MindSection` geometry.
//! Kept in the model crate so application setters and `maptool
//! verify` reject the same shapes with the same messages.

use std::collections::HashMap;

use super::{MindNode, Position, Size, MAX_NODE_AXIS, MAX_SECTIONS_PER_NODE};

/// Validate a node size candidate, returning the first violation.
pub fn node_size(size: Size) -> Result<(), String> {
    first_or_ok(node_size_violations(size))
}

/// All node-size violations for callers that report rather than
/// reject, such as `maptool verify`.
pub fn node_size_violations(size: Size) -> Vec<String> {
    let mut out = Vec::new();
    if !size.width.is_finite() || !size.height.is_finite() {
        out.push(format!(
            "node.size has non-finite component (width={}, height={})",
            size.width, size.height
        ));
    }
    if size.width.is_finite() && size.width <= 0.0 {
        out.push(format!("node.size.width is not positive ({})", size.width));
    }
    if size.height.is_finite() && size.height <= 0.0 {
        out.push(format!("node.size.height is not positive ({})", size.height));
    }
    if size.width.is_finite() && size.width > MAX_NODE_AXIS {
        out.push(format!(
            "node.size.width ({}) exceeds the {} ceiling; likely a typo (e.g. an extra zero)",
            size.width, MAX_NODE_AXIS
        ));
    }
    if size.height.is_finite() && size.height > MAX_NODE_AXIS {
        out.push(format!(
            "node.size.height ({}) exceeds the {} ceiling; likely a typo (e.g. an extra zero)",
            size.height, MAX_NODE_AXIS
        ));
    }
    out
}

/// Validate the section-count cap for one node.
pub fn section_count(node: &MindNode) -> Result<(), String> {
    if node.sections.len() > MAX_SECTIONS_PER_NODE {
        Err(format!(
            "node.sections.len()={} exceeds cap {}",
            node.sections.len(),
            MAX_SECTIONS_PER_NODE
        ))
    } else {
        Ok(())
    }
}

/// Validate one existing section's AABB against its parent node.
pub fn section_aabb(node: &MindNode, section_idx: usize) -> Result<(), String> {
    let Some(section) = node.sections.get(section_idx) else {
        return Ok(());
    };
    node_size(node.size)?;
    first_or_ok(section_aabb_violations(
        node.size,
        section_idx,
        section.offset,
        section.size,
    ))
}

/// Validate a candidate section AABB against a parent node size.
pub fn section_candidate_aabb(
    parent_size: Size,
    section_idx: usize,
    offset: Position,
    size: Option<Size>,
) -> Result<(), String> {
    node_size(parent_size)?;
    first_or_ok(section_aabb_violations(parent_size, section_idx, offset, size))
}

/// All section-AABB violations for callers that report rather than
/// reject, such as `maptool verify`.
pub fn section_aabb_violations(
    parent_size: Size,
    section_idx: usize,
    offset: Position,
    size: Option<Size>,
) -> Vec<String> {
    let mut out = Vec::new();
    if !offset.x.is_finite() || !offset.y.is_finite() {
        out.push(format!(
            "section[{}].offset has non-finite component (x={}, y={})",
            section_idx, offset.x, offset.y
        ));
    }
    if offset.x.is_finite() && offset.x < 0.0 {
        out.push(format!(
            "section[{}].offset.x is negative ({})",
            section_idx, offset.x
        ));
    }
    if offset.y.is_finite() && offset.y < 0.0 {
        out.push(format!(
            "section[{}].offset.y is negative ({})",
            section_idx, offset.y
        ));
    }
    if let Some(size) = size {
        if !size.width.is_finite() || !size.height.is_finite() {
            out.push(format!(
                "section[{}].size has non-finite component (width={}, height={})",
                section_idx, size.width, size.height
            ));
        }
        if size.width.is_finite() && size.width <= 0.0 {
            out.push(format!(
                "section[{}].size.width is not positive ({})",
                section_idx, size.width
            ));
        }
        if size.height.is_finite() && size.height <= 0.0 {
            out.push(format!(
                "section[{}].size.height is not positive ({})",
                section_idx, size.height
            ));
        }
        if parent_size.width.is_finite() && size.width.is_finite() && size.width > parent_size.width * 100.0 {
            out.push(format!(
                "section[{}].size.width ({}) is over 100× the node's width ({}); \
                 likely a typo (e.g. an extra zero)",
                section_idx, size.width, parent_size.width
            ));
        }
        if parent_size.height.is_finite()
            && size.height.is_finite()
            && size.height > parent_size.height * 100.0
        {
            out.push(format!(
                "section[{}].size.height ({}) is over 100× the node's height ({}); \
                 likely a typo (e.g. an extra zero)",
                section_idx, size.height, parent_size.height
            ));
        }
    }

    let effective = size.unwrap_or(parent_size);
    if offset.x.is_finite()
        && offset.y.is_finite()
        && effective.width.is_finite()
        && effective.height.is_finite()
    {
        let right = offset.x + effective.width;
        let bottom = offset.y + effective.height;
        if right > parent_size.width {
            out.push(format!(
                "section[{}] extends past node right edge ({} > {})",
                section_idx, right, parent_size.width
            ));
        }
        if bottom > parent_size.height {
            out.push(format!(
                "section[{}] extends past node bottom edge ({} > {})",
                section_idx, bottom, parent_size.height
            ));
        }
    }
    out
}

/// Section-channel collision warnings for one node.
pub fn section_channel_collisions(node: &MindNode) -> Vec<String> {
    let mut by_channel: HashMap<usize, Vec<usize>> = HashMap::new();
    for (idx, section) in node.sections.iter().enumerate() {
        by_channel
            .entry(section.effective_channel(idx))
            .or_default()
            .push(idx);
    }

    let mut out = Vec::new();
    for (channel, indices) in by_channel {
        if indices.len() > 1 {
            out.push(format!(
                "channel {} shared by sections {:?}; mutations targeting that channel \
                 broadcast to all listed sections — usually unintentional",
                channel, indices
            ));
        }
    }
    out
}

fn first_or_ok(messages: Vec<String>) -> Result<(), String> {
    match messages.into_iter().next() {
        Some(message) => Err(message),
        None => Ok(()),
    }
}
