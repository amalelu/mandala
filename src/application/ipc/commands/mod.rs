// SPDX-License-Identifier: MPL-2.0

//! The family registrar index: one module per command family, one
//! line per family in [`FAMILIES`] (`format/ipc.md` §Command
//! families). A new family adds its module here and its row below,
//! and touches nothing else that another family's PR touches.

mod act;
mod meta;

use super::registry::Family;

/// Every registered family. `describe` and dispatch both walk this
/// table and nothing else.
pub(super) const FAMILIES: &[Family] = &[meta::FAMILY, act::FAMILY];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ipc::registry::BOOTSTRAP_FAMILY;
    use baumhard::util::doc_fixtures::{documented_paragraph, repo_path, section_text};

    /// The family-map row for `family`: the command names its last
    /// column lists.
    fn documented_commands(family: &str) -> Vec<String> {
        let map = documented_paragraph(
            &repo_path("format/ipc.md"),
            "## Command families",
            "| family | module",
        );
        let label = if family == BOOTSTRAP_FAMILY {
            "*(bootstrap)*".to_string()
        } else {
            format!("`{family}`")
        };
        let row = map
            .lines()
            .find(|line| line.starts_with(&format!("| {label} |")))
            .unwrap_or_else(|| panic!("format/ipc.md's family map has no row for {label}"));
        row.rsplit('|')
            .nth(1)
            .expect("the row ends with its command column")
            .split(',')
            .map(|name| name.trim().trim_matches('`').to_string())
            .collect()
    }

    /// **`describe` stays byte-honest against `format/ipc.md`.** For
    /// every family this build registers, the family map lists
    /// exactly the registered commands, and the row names the module
    /// the family lives in. A family whose issue has not merged is
    /// forward contract in the document and is not checked here,
    /// because `describe` reflects only what the binary registers.
    #[test]
    fn test_every_registered_family_matches_its_family_map_row() {
        for family in FAMILIES {
            let registered: Vec<&str> = family.commands.iter().map(|c| c.name).collect();
            assert_eq!(
                registered,
                documented_commands(family.name),
                "family `{}`: the registry and format/ipc.md's family map disagree",
                family.name
            );
        }
    }

    /// Each registered command has its reference in the document: a
    /// section of its own for the bootstrap pair, a bullet in its
    /// family's section otherwise. Every parameter and result field
    /// it publishes is named there, so `describe` cannot grow a
    /// parameter the document does not mention.
    #[test]
    fn test_every_registered_command_and_field_is_documented() {
        let doc = repo_path("format/ipc.md");
        for family in FAMILIES {
            for command in family.commands {
                let reference = if family.name == BOOTSTRAP_FAMILY {
                    section_text(&doc, &format!("### `{}`", command.name))
                } else {
                    let heading = std::fs::read_to_string(&doc)
                        .expect("format/ipc.md is readable")
                        .lines()
                        .find(|line| line.starts_with(&format!("## `{}` — ", family.name)))
                        .unwrap_or_else(|| panic!("format/ipc.md has no `## `{}` — …` section", family.name))
                        .to_string();
                    section_text(&doc, &heading)
                };
                // A bootstrap command's own heading is its reference,
                // and `section_text` has already refused a missing one.
                assert!(
                    family.name == BOOTSTRAP_FAMILY || reference.contains(&format!("`{}`", command.name)),
                    "format/ipc.md does not reference `{}` in its family's section",
                    command.name
                );
                let fields = command.params.iter().map(|p| p.name);
                for field in fields.chain(command.result.iter().map(|r| r.name)) {
                    assert!(
                        reference.contains(&format!("`{field}`")),
                        "format/ipc.md's reference for `{}` does not name `{field}`",
                        command.name
                    );
                }
            }
        }
    }
}
