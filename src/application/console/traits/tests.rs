//! Unit tests for the trait dispatcher's data types — color value
//! parsing, outcome helper, and selection materialization.

use super::*;
use crate::application::console::constants::{VAR_ACCENT, VAR_EDGE, VAR_FG};
use crate::application::document::SelectionState;

#[test]
fn test_parse_hex_ok() {
    assert_eq!(ColorValue::parse("#123").unwrap(), ColorValue::Hex("#123".into()));
    assert_eq!(
        ColorValue::parse("#009c15").unwrap(),
        ColorValue::Hex("#009c15".into())
    );
    assert_eq!(
        ColorValue::parse("#009c15ff").unwrap(),
        ColorValue::Hex("#009c15ff".into())
    );
}

#[test]
fn test_parse_hex_rejects_bad_length() {
    assert!(ColorValue::parse("#12").is_err());
    assert!(ColorValue::parse("#12345").is_err());
    assert!(ColorValue::parse("#zzzzzz").is_err());
}

#[test]
fn test_parse_var_tokens() {
    assert_eq!(ColorValue::parse("accent").unwrap(), ColorValue::Var(VAR_ACCENT));
    assert_eq!(ColorValue::parse("ACCENT").unwrap(), ColorValue::Var(VAR_ACCENT));
    assert_eq!(ColorValue::parse("fg").unwrap(), ColorValue::Var(VAR_FG));
    assert_eq!(ColorValue::parse("edge").unwrap(), ColorValue::Var(VAR_EDGE));
}

#[test]
fn test_parse_reset() {
    assert_eq!(ColorValue::parse("reset").unwrap(), ColorValue::Reset);
}

#[test]
fn test_parse_unknown_is_error() {
    assert!(ColorValue::parse("bogus").is_err());
}

#[test]
fn test_outcome_applied_helper() {
    assert_eq!(Outcome::applied(true), Outcome::Applied);
    assert_eq!(Outcome::applied(false), Outcome::Unchanged);
}

#[test]
fn test_selection_targets_for_each_variant() {
    use crate::application::document::{EdgeRef, PortalRef};
    assert!(selection_targets(&SelectionState::None).is_empty());

    let ids = vec!["a".to_string(), "b".to_string()];
    let out = selection_targets(&SelectionState::Multi(ids.clone()));
    assert_eq!(out.len(), 2);

    let er = EdgeRef::new("a", "b", "cross_link");
    let out = selection_targets(&SelectionState::Edge(er));
    assert!(matches!(out.as_slice(), [TargetId::Edge(_)]));

    let pr = PortalRef {
        label: "A".into(),
        endpoint_a: "x".into(),
        endpoint_b: "y".into(),
    };
    let out = selection_targets(&SelectionState::Portal(pr));
    assert!(matches!(out.as_slice(), [TargetId::Portal(_)]));
}
