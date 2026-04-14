//! `ColorValue` — the parsed form of a color spec from the kv console.
//! Resolution of `Var` happens downstream at scene-build time via
//! `baumhard::util::color::resolve_var`; the model's color fields are
//! `String`, so passing a structured `ColorValue` through the traits
//! lets each trait impl decide what `Reset` means without
//! pre-resolving `var(--accent)`.

use crate::application::console::constants::{VAR_ACCENT, VAR_EDGE, VAR_FG};

/// A parsed color value from the kv-form console.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorValue {
    /// Literal hex string, e.g. `"#009c15"` or `"#009c15ff"`. Stored
    /// verbatim so it lands directly in the model's color field.
    Hex(String),
    /// Theme variable reference, e.g. `VAR_ACCENT`. Written into the
    /// model as `"var(--accent)"` so the scene builder resolves it
    /// against the canvas's theme_variables.
    Var(&'static str),
    /// Clear any local override — the component falls back to its
    /// natural default. What "natural default" means is per-trait.
    Reset,
}

impl ColorValue {
    /// Parse `s` into a ColorValue. Accepts:
    /// - `"#rrggbb"` / `"#rrggbbaa"` / `"#rgb"` / `"#rgba"` — as hex
    /// - `"accent"` / `"edge"` / `"fg"` / `"bg"` — as well-known vars
    /// - `"reset"` — as Reset
    ///
    /// Returns `Err(msg)` on unrecognised input. Callers report the
    /// error through the per-kv outcome.
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if let Some(rest) = t.strip_prefix('#') {
            // Shape-check: 3 / 4 / 6 / 8 hex digits.
            let valid = matches!(rest.len(), 3 | 4 | 6 | 8)
                && rest.chars().all(|c| c.is_ascii_hexdigit());
            if !valid {
                return Err(format!("invalid hex color: '{}'", s));
            }
            return Ok(ColorValue::Hex(t.to_string()));
        }
        match t.to_ascii_lowercase().as_str() {
            "accent" => Ok(ColorValue::Var(VAR_ACCENT)),
            "edge" => Ok(ColorValue::Var(VAR_EDGE)),
            "fg" => Ok(ColorValue::Var(VAR_FG)),
            "reset" => Ok(ColorValue::Reset),
            other => Err(format!("unknown color '{}'", other)),
        }
    }

    /// Render as the string form the model stores — `"#rrggbb"` for
    /// `Hex`, `"var(--...)"` for `Var`. `Reset` has no single string
    /// form and the caller must decide what it means; this method
    /// is only meaningful on `Hex` and `Var`.
    pub fn as_model_string(&self) -> Option<String> {
        match self {
            ColorValue::Hex(s) => Some(s.clone()),
            ColorValue::Var(v) => Some((*v).to_string()),
            ColorValue::Reset => None,
        }
    }
}
