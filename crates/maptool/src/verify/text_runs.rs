//! Text-run invariants: ordered, non-overlapping, within text bounds.
//!
//! `start` and `end` are measured in **Unicode code points** (Rust's
//! `char` count), matching how `ColorFontRegions` interprets them in the
//! tree builder. Not bytes, not grapheme clusters — code points.

use baumhard::mindmap::model::MindMap;

use super::Violation;

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for node in map.nodes.values() {
        if node.text_runs.is_empty() {
            continue;
        }

        let total = node.text.chars().count();
        let mut prev_end: Option<usize> = None;

        for (i, run) in node.text_runs.iter().enumerate() {
            if run.start >= run.end {
                out.push(Violation {
                    category: "text_runs",
                    location: node.id.clone(),
                    message: format!(
                        "run[{}] has start {} not less than end {}",
                        i, run.start, run.end
                    ),
                });
                continue;
            }

            if run.end > total {
                out.push(Violation {
                    category: "text_runs",
                    location: node.id.clone(),
                    message: format!(
                        "run[{}] end {} exceeds text length {} (code points)",
                        i, run.end, total
                    ),
                });
            }

            if let Some(p) = prev_end {
                if run.start < p {
                    out.push(Violation {
                        category: "text_runs",
                        location: node.id.clone(),
                        message: format!(
                            "run[{}] overlaps previous run (start {} < previous end {})",
                            i, run.start, p
                        ),
                    });
                }
            }
            prev_end = Some(run.end);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::model::TextRun;
    use crate::verify::test_helpers::node;

    fn run(start: usize, end: usize) -> TextRun {
        TextRun {
            start,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14,
            color: "#ffffff".into(),
            hyperlink: None,
        }
    }

    #[test]
    fn empty_runs_clean() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.text = "Hello".into();
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn valid_runs_clean() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.text = "Hello world".into();
        n.text_runs = vec![run(0, 5), run(6, 11)];
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn overlapping_runs_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.text = "Hello world".into();
        n.text_runs = vec![run(0, 5), run(3, 8)];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "text_runs" && x.message.contains("overlap")));
    }

    #[test]
    fn out_of_bounds_runs_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.text = "Hi".into();
        n.text_runs = vec![run(0, 100)];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "text_runs" && x.message.contains("exceeds")));
    }

    #[test]
    fn inverted_run_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.text = "Hello".into();
        n.text_runs = vec![run(3, 3)];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "text_runs" && x.message.contains("not less than")));
    }
}
