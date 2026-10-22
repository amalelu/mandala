// SPDX-License-Identifier: MPL-2.0

//! `ConsoleState` shape smoke tests.

use crate::application::console::ConsoleState;

#[test]
fn test_console_state_open_seeds_history_and_resets_cursor() {
    let history = vec!["help".to_string(), "anchor set from auto".to_string()];
    let open = ConsoleState::open(history.clone());
    assert!(open.is_open());
    assert!(!ConsoleState::Closed.is_open());
    match open {
        ConsoleState::Open {
            history: h,
            input,
            cursor,
            ..
        } => {
            assert_eq!(h, history);
            assert_eq!(input, "");
            assert_eq!(cursor, 0);
        }
        _ => panic!("expected Open"),
    }
}
