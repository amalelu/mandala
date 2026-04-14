//! `ConsoleState` shape smoke tests — verifies the open/closed
//! constructors set up the expected fields.

use crate::application::console::ConsoleState;

#[test]
fn test_console_state_open_is_not_closed() {
    let open = ConsoleState::open(Vec::new());
    assert!(open.is_open());
    let closed = ConsoleState::Closed;
    assert!(!closed.is_open());
}

#[test]
fn test_console_state_open_seeded_with_history() {
    let history = vec!["help".to_string(), "anchor set from auto".to_string()];
    match ConsoleState::open(history.clone()) {
        ConsoleState::Open { history: h, input, cursor, .. } => {
            assert_eq!(h, history);
            assert_eq!(input, "");
            assert_eq!(cursor, 0);
        }
        _ => panic!("expected Open"),
    }
}
