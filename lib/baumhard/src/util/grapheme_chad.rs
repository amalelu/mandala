use log::error;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn slice_to_newline(s: &str, byte_index: usize) -> &str {
    let end_byte_index = s[byte_index..]
        .find('\n')
        .map_or(s.len(), |i| byte_index + i);

    &s[byte_index..end_byte_index]
}

/// For each grapheme-cluster in source, replace the corresponding grapheme-cluster in target
/// Or if source is larger than target, append the grapheme-clusters that does not have a corresponding one
/// Ignores everything after first newline
/// Returns: If the string that was inserted was larger than the original string
/// Some(end_index_of_line, num_grapheme_clusters_added) which can be used to adjust regions
/// otherwise, if the inserted string was smaller or equal None
pub fn replace_graphemes_until_newline(
    target: &mut String,
    g_index: usize,
    source: &str,
) -> Option<(usize, usize)> {
    let insert_num_graphemes = count_grapheme_clusters(source);
    let b_index = find_byte_index_of_grapheme(target, g_index).unwrap_or(target.len());

    let line_section = slice_to_newline(target, b_index);

    let target_line_num_graphemes = count_grapheme_clusters(line_section);
    let end_of_target_line_idx = b_index + line_section.len();

    if insert_num_graphemes >= target_line_num_graphemes {
        // We can basically cut away this whole region and then insert our string
        replace_substring(target, b_index, end_of_target_line_idx, source);
        Some((g_index, insert_num_graphemes - target_line_num_graphemes))
    } else {
        // We need to cut away a part between index..insert_num_graphemes, and then insert our string
        replace_substring(
            target,
            b_index,
            find_byte_index_of_grapheme(target, g_index + insert_num_graphemes).unwrap(),
            source,
        );
        None
    }
}

pub fn find_byte_index_of_char(s: &str, index: usize) -> Option<usize> {
    s.char_indices().nth(index).map(|(byte_idx, _)| byte_idx)
}

pub fn find_byte_index_of_grapheme(s: &str, index: usize) -> Option<usize> {
    let mut byte_index = 0;
    for (i, grapheme) in s.graphemes(true).enumerate() {
        if i == index {
            return Some(byte_index);
        }
        byte_index += grapheme.len();
    }
    None // Return None if index is out of bounds
}

/// Finds the index immediately after the nth grapheme
fn find_index_after_nth_grapheme(str: &str, n: usize) -> Option<usize> {
    // Graphemes method provides an iterator over the grapheme clusters
    let mut graphemes = str.graphemes(true);

    // Skip n graphemes and take the next one to find the boundary after the nth grapheme
    let skipped_graphemes = graphemes.by_ref().take(n + 1).collect::<Vec<&str>>();

    // If the number of graphemes collected is less than or equal to n, we've reached or exceeded the end of the string
    if skipped_graphemes.len() <= n {
        return None; // Return None if we cannot move n graphemes into the string
    }
    // Calculate the byte index: the sum of the lengths (in bytes) of all taken graphemes
    let byte_index = skipped_graphemes.iter().map(|g| g.len()).sum();

    Some(byte_index)
}

pub fn nth_grapheme_cluster_byte_index(s: &str, n: usize) -> Option<usize> {
    let mut index = 0;
    for (i, (start, _)) in s.grapheme_indices(true).enumerate() {
        if i == n {
            return Some(start);
        }
        index = start;
    }
    if n == 0 {
        return Some(index);
    }
    None
}

fn replace_substring(s: &mut String, i: usize, n: usize, source: &str) {
    let mut bytes = s.as_bytes().to_vec(); // Convert the String to a Vec<u8>
    let source_bytes = source.as_bytes();

    // Remove the specified range
    bytes.drain(i..n);

    // Insert the new bytes
    bytes.splice(i..i, source_bytes.iter().cloned());

    // Safely convert Vec<u8> back to String
    if let Ok(modified_string) = String::from_utf8(bytes) {
        *s = modified_string;
    } else {
        // Handle invalid UTF-8 conversion, if necessary
        error!("Failed to convert bytes to UTF-8 String.");
    }
}

pub fn split_off_graphemes(original: &mut String, at: usize) -> String {
    let graphemes = original.graphemes(true).collect::<Vec<&str>>();

    if at >= graphemes.len() {
        return original.split_off(original.len());
    }

    let (left, right) = graphemes.split_at(at);
    let right_str = right.concat();

    *original = left.concat();
    right_str
}

pub fn count_number_lines(s: &str) -> usize {
    s.as_bytes().iter().filter(|&&c| c == b'\n').count() + 1
}

pub fn find_nth_line_grapheme_range(s: &str, n: usize) -> Option<(usize, usize)> {
    if s.len() == 0 {
        return None;
    }
    let mut line_head = 0;
    let mut last_line_start = 0;
    let mut new_line: bool = true;
    for (idx, graph) in s.graphemes(true).enumerate() {
        if new_line {
            last_line_start = idx;
            new_line = false;
        }
        if graph == "\n" || graph == "" {
            if line_head == n {
                // So it's time to move the line head up one tick
                // but if the head is currently at n, then this is the last
                // index in the line
                return Some((last_line_start, idx));
            }
            // otherwise
            new_line = true;
            line_head += 1;
        }
    }
    if line_head < n || (line_head == n && new_line) {
        return None;
    }
    Some((last_line_start, s.graphemes(true).count()))
}

pub fn find_nth_line_byte_range(s: &str, n: usize) -> Option<(usize, usize)> {
    if s.len() == 0 {
        return None;
    }
    let mut line_head = 0;
    let mut last_line_start = 0;
    let mut new_line: bool = true;
    for (idx, ch) in s.char_indices() {
        if new_line {
            last_line_start = idx;
            new_line = false;
        }
        if ch == '\n' {
            if line_head == n {
                // So it's time to move the line head up one tick
                // but if the head is currently at n, then this is the last
                // index in the line
                return Some((last_line_start, idx));
            }
            // otherwise
            new_line = true;
            line_head += 1;
        }
    }
    if line_head < n || (line_head == n && new_line) {
        return None;
    }
    Some((last_line_start, s.len()))
}

pub fn insert_new_lines(s: &mut String, n: usize) {
    let newlines = "\n".repeat(n);
    s.push_str(&newlines);
}

pub fn push_spaces(s: &mut String, n: usize) {
    let spaces = " ".repeat(n);
    s.push_str(&spaces);
}

pub fn insert_spaces(s: &mut String, idx: usize, n: usize) {
    let spaces = " ".repeat(n);
    let maybe = nth_grapheme_cluster_byte_index(s, idx);
    if let Some(T) = maybe {
        s.insert_str(T, &spaces);
    } else {
        push_spaces(s, n);
    }
}

pub fn count_grapheme_clusters(s: &str) -> usize {
    s.graphemes(true).count()
}

pub fn delete_back_unicode(s: &mut String, n: usize) {
    let mut char_count = 0;
    let mut grapheme_count = 0;

    for grapheme in UnicodeSegmentation::graphemes(s.as_str(), true).rev() {
        grapheme_count += 1;
        if grapheme_count > n {
            break;
        }
        char_count += grapheme.len();
    }
    if grapheme_count <= n {
        s.clear(); // If there are fewer or equal graphemes than n, clear the string
        return;
    }
    let new_len = s.len() - char_count;
    s.truncate(new_len);
}

pub fn delete_front_unicode(s: &mut String, n: usize) {
    let mut char_count = 0;
    let mut grapheme_count = 0;

    for grapheme in UnicodeSegmentation::graphemes(s.as_str(), true) {
        grapheme_count += 1;
        char_count += grapheme.len();

        if grapheme_count >= n {
            break;
        }
    }
    if grapheme_count < n {
        s.clear(); // If there are fewer graphemes than n, clear the string
        return;
    }
    // Remove the first n graphemes
    s.drain(0..char_count);
}

#[cfg(test)]
mod test {

}
