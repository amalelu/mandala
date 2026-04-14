//! `Outcome` — the result type for a single capability-trait call.
//! Aggregated by the dispatcher into a per-kv report line.

/// Outcome of a single trait call.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The setter ran and actually changed something.
    Applied,
    /// The setter ran but the value matched the current state, so
    /// nothing changed. Not an error — distinguishable from
    /// `Applied` so "already set" feedback is possible.
    Unchanged,
    /// The target doesn't implement this trait (e.g. `text=` on a
    /// portal). The dispatcher reports this per-pair so `color
    /// bg=#fff text=accent` applies the `bg` to a portal while the
    /// `text` pair is reported as not-applicable.
    NotApplicable,
    /// The value was rejected by the target (e.g. a negative font
    /// size).
    Invalid(String),
}

impl Outcome {
    pub fn applied(changed: bool) -> Self {
        if changed { Outcome::Applied } else { Outcome::Unchanged }
    }
}
