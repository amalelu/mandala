// SPDX-License-Identifier: MPL-2.0

//! The keybind **declaration surface**: the machinery behind the one
//! table in [`super::config`] that declares how every [`Action`]
//! reaches `keybinds.json`.
//!
//! ## Why this exists
//!
//! A bindable Action used to be declared in three hand-synced places
//! — a [`super::KeybindConfig`] struct field, a `Default` entry, and a
//! row in `resolve()`. A field missing from `resolve()` compiled
//! cleanly and the binding was silently dead, which is a bug that had
//! already shipped more than once (`Action::SetBorderPreview` had a
//! field and no row; `Action::CycleBorderPreset` and
//! `Action::ToggleBorderVisible` documented themselves as bindable
//! and had neither).
//!
//! [`keybind_surface!`] collapses the three into one row per Action
//! and makes the fourth thing — *did you decide at all?* — a compile
//! error: alongside the struct, the `Default` impl and the resolve
//! step, the macro emits a `match` over every `ActionKind`, and the
//! recognized-key set that the loader's unknown-key warning consults
//! is built by walking that match. A new `Action` variant that
//! appears in no section of the table fails the match's
//! exhaustiveness check, in an ordinary `cargo build`.
//!
//! ## The two binding surfaces
//!
//! - **Simple** — a `Vec<String>` of key combos, for unit variants:
//!   `"undo": ["Ctrl+Z", "Undo"]`.
//! - **Parametric** — a `Vec<ParametricBinding>`, for payload-carrying
//!   variants: `"set_color": [{ "combo": "F1", "args": ["bg", "#fafafa"] }]`.
//!
//! Which surface a variant lands on is not a choice: the `simple`
//! section names the variant with no payload, so listing a
//! payload-carrying variant there does not compile, and the
//! `parametric` section requires at least one payload field. Unit
//! variants therefore always take the ergonomic string-list form and
//! payload variants always take the args form.
//!
//! ## Payload conversion
//!
//! Positional `args` reach the variant's fields through [`ArgValue`]:
//! `String` fields take the argument verbatim, typed fields
//! ([`ColorAxis`], [`FontSlot`], ...) go through their strum-derived
//! `FromStr`. A field whose type has no `ArgValue` impl does not
//! compile, which is the seam a new typed payload extends —
//! `impl_arg_value_via_from_str!` takes the type name and nothing
//! else.
//!
//! ## Payload field order
//!
//! *Which* field each arg reaches is the row's field order, and the
//! generated constructor cannot enforce it: a struct expression is
//! order-free, so a transposed row compiles and swaps the user's
//! arguments. [`keybind_field_order_check!`] closes that at build
//! time by comparing the row's names against the `Action`
//! declaration's own — published by
//! `mandala_derive::PayloadFieldNames` — so a transposition is
//! `error[E0080]` on the row rather than a runtime surprise.

use log::warn;
use serde::{Deserialize, Serialize};

use super::action::{Action, BorderPreviewTargetKind, ColorAxis, FontSlot, ZoomBound};
use super::bind::KeyBind;

/// One binding for a parametric Action — a key combo plus the
/// positional payload args the resolve step feeds into the
/// variant's payload. Free-form `String` args; typed validation
/// happens at resolve time via [`ArgValue`] (unparseable values emit
/// a warn-log and the binding is skipped) and again in the dispatch
/// arm for the values that stay stringly-typed.
///
/// Args are positional rather than keyed so the same shape covers
/// single-payload (1 arg, e.g. `set_edge_body_glyph(["dash"])`),
/// from/to (2 args, e.g. `set_edge_anchor(["top", "auto"])`), and
/// field/value (2 args) variants. Per-variant arg names come from
/// the payload field names in the `keybind_surface!` table and are
/// quoted back at the user in the warn-log when the count is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParametricBinding {
    pub combo: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Where an [`Action`] lives on the `keybinds.json` surface — the
/// value side of the `ActionKind` → key match that
/// [`keybind_surface!`] generates, and the reason that match is worth
/// having rather than a bare `Option<&str>`: the two bound cases take
/// different JSON value shapes, and a caller checking one against the
/// other (a test, a future help surface) needs to know which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindSurface {
    /// A `Vec<String>` field of key combos — `"undo": ["Ctrl+Z"]`.
    /// Unit Action variants only.
    Simple(&'static str),
    /// A `Vec<ParametricBinding>` field —
    /// `"set_color": [{ "combo": …, "args": [ … ] }]`.
    /// Payload-carrying Action variants only.
    Parametric(&'static str),
    /// The Action has no key surface. The reason is recorded in the
    /// table's `unbindable` section and published on the generated
    /// `bind_surface`.
    Unbindable,
}

impl BindSurface {
    /// The `keybinds.json` key, or `None` for an unbindable Action.
    pub fn field(self) -> Option<&'static str> {
        match self {
            BindSurface::Simple(field) | BindSurface::Parametric(field) => Some(field),
            BindSurface::Unbindable => None,
        }
    }
}

/// Conversion from one positional `args` entry to a payload field.
///
/// Implemented for `String` (verbatim) and for each typed payload
/// enum (via its strum-derived `FromStr`). The trait is what lets
/// the `keybind_surface!` table name a variant's payload fields
/// without also restating their types: the field type picks the
/// impl, and a type with no impl is a compile error rather than a
/// silently stringly-typed payload.
pub trait ArgValue: Sized {
    /// Convert one raw argument string, or `None` when the argument
    /// is not a valid value for this field. `None` makes the resolve
    /// step skip the binding with a warn-log — a user-config typo
    /// never panics and never disables the rest of the config.
    fn parse_arg(raw: &str) -> Option<Self>;
}

impl ArgValue for String {
    fn parse_arg(raw: &str) -> Option<Self> {
        Some(raw.to_string())
    }
}

/// Implement [`ArgValue`] for types that already round-trip through
/// `FromStr` — the strum-derived payload enums. One line per type;
/// this is the extension point a new typed Action payload uses.
macro_rules! impl_arg_value_via_from_str {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl ArgValue for $ty {
                fn parse_arg(raw: &str) -> Option<Self> {
                    raw.parse().ok()
                }
            }
        )+
    };
}

impl_arg_value_via_from_str!(ColorAxis, BorderPreviewTargetKind, FontSlot, ZoomBound);

/// Resolve every binding for one parametric variant.
///
/// `arg_names` comes from the payload field names in the
/// `keybind_surface!` table, so the arity can neither be passed
/// wrongly nor drift from the builder closure, and the warn-log can
/// quote the names the user's `args` array should have carried.
///
/// Three distinct skips, each logged: an unparseable combo, an `args`
/// array of the wrong length, and an argument that is not a valid
/// value for the field it feeds. None of them panics and none of them
/// discards the rest of the config.
pub fn push_parametric<F>(
    binds: &mut Vec<(Action, KeyBind)>,
    field: &str,
    arg_names: &[&str],
    bindings: &[ParametricBinding],
    build: F,
) where
    F: Fn(&[String]) -> Option<Action>,
{
    for binding in bindings {
        let key = match KeyBind::parse(&binding.combo) {
            Ok(k) => k,
            Err(e) => {
                warn!("keybinds: skipping invalid keybind '{}': {}", binding.combo, e);
                continue;
            }
        };
        if binding.args.len() != arg_names.len() {
            warn!(
                "keybinds: skipping {} binding '{}': expected {} arg(s) [{}], got {}",
                field,
                binding.combo,
                arg_names.len(),
                arg_names.join(", "),
                binding.args.len(),
            );
            continue;
        }
        match build(&binding.args) {
            Some(action) => binds.push((action, key)),
            None => warn!(
                "keybinds: skipping {} binding '{}': args {:?} are not valid values for [{}]",
                field,
                binding.combo,
                binding.args,
                arg_names.join(", "),
            ),
        }
    }
}

/// Build one parametric `Action` from a positional `args` slice.
/// Two rules, one per payload shape — named-field variants and tuple
/// variants — because a `macro_rules!` repetition cannot itself
/// branch on the shape of what it captured. Invoked by
/// [`keybind_surface!`], not by hand.
macro_rules! keybind_build_action {
    ( $Variant:ident { $($field:ident),+ $(,)? }, $args:expr ) => {
        match $args {
            [$($field),+] => ::std::option::Option::Some(
                $crate::application::keybinds::Action::$Variant {
                    $( $field: $crate::application::keybinds::surface::ArgValue::parse_arg($field)? ),+
                }
            ),
            _ => ::std::option::Option::None,
        }
    };
    ( $Variant:ident ( $($field:ident),+ $(,)? ), $args:expr ) => {
        match $args {
            [$($field),+] => ::std::option::Option::Some(
                $crate::application::keybinds::Action::$Variant(
                    $( $crate::application::keybinds::surface::ArgValue::parse_arg($field)? ),+
                )
            ),
            _ => ::std::option::Option::None,
        }
    };
}

/// The payload field names of one parametric variant, in positional
/// order — the arity and the warn-log's argument list in one value.
/// Same two-rule shape as [`keybind_build_action!`].
macro_rules! keybind_arg_names {
    ( { $($field:ident),+ $(,)? } ) => { &[$(stringify!($field)),+] };
    ( ( $($field:ident),+ $(,)? ) ) => { &[$(stringify!($field)),+] };
}

/// `&str` equality usable in a `const` context, where `==` is not.
///
/// Inputs: two string slices. Cost: one length comparison and at
/// most `len` byte comparisons, all at compile time — nothing of
/// this survives into the binary. Byte-wise rather than
/// grapheme-aware on purpose: both operands are Rust identifiers
/// rendered by `stringify!`, so they are ASCII by the grammar that
/// produced them, and `baumhard::util::grapheme_chad` is not
/// `const`.
pub const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Whether two field-name lists are equal *and in the same order* —
/// the comparison [`keybind_field_order_check!`] runs at compile
/// time. Cost: as [`str_eq`], summed over the shorter list; compile
/// time only.
pub const fn field_names_eq(a: &[&str], b: &[&str]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if !str_eq(a[i], b[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// Assert, at compile time, that one parametric row lists its
/// payload fields in the order the `Action` variant declares them.
///
/// ## The defect this closes
///
/// For a named-field variant the field order in the row *is* the
/// positional `args` contract: `SetColor { axis, value }` means
/// `args[0]` fills `axis`. But the row expands into a struct
/// expression, and a struct expression is order-free — so writing
/// the row `{ value, axis }` compiles, resolves, and hands the
/// user's first argument to the second field. A typed field trips
/// over the swap by accident (`"#fafafa"` is not a `ColorAxis`); an
/// all-`String` payload does not, and `AddSection { at, text }`
/// written `{ text, at }` passed the entire test suite.
///
/// The sentinel table in the keybinds tests pins the *payload
/// values* a row must produce, which catches the same defect — but
/// only once a human has written that row's sentinel entry. This
/// check needs nothing written: it compares the row's field names
/// against `action_payload_fields`, which
/// `mandala_derive::PayloadFieldNames` reads off the `Action`
/// declaration. Two independent sources, so it is not a mirror, and
/// a brand-new transposed row fails to compile before any test
/// exists for it.
///
/// ## Why tuple variants expand to nothing
///
/// The second rule is deliberately empty. A tuple row's names
/// (`SetEdgeBodyGlyph(glyph)`) are local bindings invented by the
/// table, not declared anywhere, and the pattern that binds them and
/// the constructor that consumes them are the *same* macro
/// repetition — so there is no second order for them to disagree
/// with. There is nothing to check, rather than a check omitted.
macro_rules! keybind_field_order_check {
    ( $Variant:ident { $($field:ident),+ $(,)? } ) => {
        const _: () = assert!(
            $crate::application::keybinds::surface::field_names_eq(
                $crate::application::keybinds::surface::keybind_arg_names!({ $($field),+ }),
                $crate::application::keybinds::action::action_payload_fields::$Variant,
            ),
            concat!(
                "keybind_surface!: the `",
                stringify!($Variant),
                "` row lists its payload fields in an order the `Action` declaration does not \
                 have. Every binding for it would hand the user's positional `args` to the wrong \
                 fields. Reorder the row to match the variant.",
            ),
        );
    };
    ( $Variant:ident ( $($field:ident),+ $(,)? ) ) => {};
}

/// Declare the entire `keybinds.json` surface from one table.
///
/// Emits, from a single row per [`Action`] variant:
///
/// - the [`super::KeybindConfig`] struct field (with the row's doc
///   comment, which is the user-facing documentation of that JSON
///   key),
/// - its `Default` entry,
/// - its row in the resolve step, including the payload builder and
///   the arity for parametric variants,
/// - its arm in `bind_surface`, the `ActionKind` → [`BindSurface`]
///   match that the recognized-key set is built from.
///
/// The last one is the forcing function: the match is exhaustive over
/// `ActionKind`, so a variant absent from every section of the table
/// fails to compile.
///
/// ## Sections
///
/// ```ignore
/// keybind_surface! {
///     simple {
///         /// Doc comment becomes the struct field's documentation.
///         Undo => undo = ["Ctrl+Z", "Undo"],
///         JumpToRoot => jump_to_root = [],       // default unbound
///     }
///     parametric {
///         SetEdgeAnchor { from, to } => set_edge_anchor,
///         SetEdgeBodyGlyph(glyph) => set_edge_body_glyph,
///     }
///     unbindable {
///         // Variant => the reason it has no key surface. The reason
///         // is required, and is published as documentation on the
///         // generated `bind_surface`.
///         ReparentToTarget => "the payload is a hit-test result",
///     }
///     extra {
///         /// Non-Action config keys — style and the id-keyed
///         /// binding maps. Written as ordinary field declarations
///         /// with their default expression.
///         pub console_font_size: f32 = 16.0,
///     }
/// }
/// ```
///
/// All four sections are required and must appear in this order;
/// each may be empty.
///
/// ## What the expansion site must have in scope
///
/// The two helper macros and [`push_parametric`] are reached by
/// absolute path, so they need no import. The rest of the emitted
/// code speaks the config module's own vocabulary and expects it
/// imported there: `Action`, `ActionKind`, `KeyBind`,
/// `ParametricBinding`, serde's `Serialize` / `Deserialize`, and
/// strum's `IntoEnumIterator` (for `ActionKind::iter()`). This macro
/// has exactly one call site — [`super::config`] — and lives next to
/// it; a second one would be a sign the surface had grown a twin.
macro_rules! keybind_surface {
    (
        simple {
            $(
                $(#[$smeta:meta])*
                $SimpleVariant:ident => $simple_field:ident = [ $($simple_default:literal),* $(,)? ]
            ),* $(,)?
        }
        parametric {
            $(
                $(#[$pmeta:meta])*
                $ParamVariant:ident $param_payload:tt => $param_field:ident
            ),* $(,)?
        }
        unbindable {
            $(
                $UnboundVariant:ident => $unbound_reason:literal
            ),* $(,)?
        }
        extra {
            $(
                $(#[$emeta:meta])*
                pub $extra_field:ident : $extra_ty:ty = $extra_default:expr
            ),* $(,)?
        }
    ) => {
        /// The raw, user-editable config — **generated** from the
        /// `keybind_surface!` table that declares it, which is the
        /// single place a bindable [`Action`] is named on the config
        /// side (see [`super::surface`]). Every action field is a
        /// list so users can assign several keys to the same action
        /// (e.g. `Ctrl+Z` and the `Undo` media key both mapped to
        /// `Undo`). Fields default via serde, so a partial config
        /// only has to mention what it overrides.
        ///
        /// Unrecognized top-level keys are kept out of the type on
        /// purpose: they are reported by
        /// [`KeybindConfig::unknown_top_level_keys`] and warned about
        /// at load, rather than rejected — one stale key must not
        /// cost the user the rest of their config. That tolerance
        /// covers stale *keys* only; a malformed *value* under a
        /// recognized key still fails the whole parse and costs the
        /// user the layer, which
        /// [`KeybindConfig::unknown_top_level_keys`] names and issue
        /// #129 tracks.
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(default)]
        pub struct KeybindConfig {
            $( $(#[$smeta])* pub $simple_field: Vec<String>, )*
            $( $(#[$pmeta])* pub $param_field: Vec<ParametricBinding>, )*
            $( $(#[$emeta])* pub $extra_field: $extra_ty, )*
        }

        impl Default for KeybindConfig {
            fn default() -> Self {
                KeybindConfig {
                    $( $simple_field: vec![ $( $simple_default.into() ),* ], )*
                    // Parametric bindings have no defaults: there is
                    // no universally sensible `from=top to=bottom`,
                    // so users opt in per binding.
                    $( $param_field: Vec::new(), )*
                    $( $extra_field: $extra_default, )*
                }
            }
        }

        impl KeybindConfig {
            /// Parse every binding string in the table into concrete
            /// `(Action, KeyBind)` pairs, appending to `binds`.
            /// Generated from the `keybind_surface!` table; the
            /// simple surface is walked before the parametric one,
            /// so a combo claimed by both resolves to the simple
            /// binding (first match wins — see
            /// [`super::ResolvedKeybinds::action_for_context`]).
            ///
            /// Any binding that fails to parse is logged and skipped
            /// so a single typo doesn't break the entire config.
            fn push_action_binds(&self, binds: &mut Vec<(Action, KeyBind)>) {
                let simple: &[(Action, &Vec<String>)] = &[
                    $( (Action::$SimpleVariant, &self.$simple_field), )*
                ];
                for (action, strings) in simple {
                    for s in *strings {
                        match KeyBind::parse(s) {
                            Ok(k) => binds.push((action.clone(), k)),
                            Err(e) => log::warn!("keybinds: skipping invalid keybind '{}': {}", s, e),
                        }
                    }
                }
                $(
                    $crate::application::keybinds::surface::push_parametric(
                        binds,
                        stringify!($param_field),
                        $crate::application::keybinds::surface::keybind_arg_names!($param_payload),
                        &self.$param_field,
                        |args: &[String]| {
                            $crate::application::keybinds::surface::keybind_build_action!(
                                $ParamVariant $param_payload, args
                            )
                        },
                    );
                )*
            }

            /// Every top-level key `keybinds.json` recognizes: one
            /// per bindable Action, plus the style and binding-map
            /// keys. Built by walking [`bind_surface`] over
            /// `ActionKind::iter()`, so the recognized set cannot
            /// drift from the struct — both come from the table.
            ///
            /// Allocates one `Vec` of static strings and walks every
            /// Action variant once; called once per config load.
            pub fn known_keys() -> Vec<&'static str> {
                ActionKind::iter()
                    .filter_map(|kind| bind_surface(kind).field())
                    .chain(EXTRA_KEYS.iter().copied())
                    .collect()
            }
        }

        /// Where each [`Action`] variant lives on the
        /// `keybinds.json` surface. Variants that deliberately have
        /// no key surface, and why:
        $(
            #[doc = concat!("- [`Action::", stringify!($UnboundVariant), "`] — ", $unbound_reason)]
        )*
        ///
        /// Generated by `keybind_surface!`. The match is exhaustive
        /// over `ActionKind`: a new Action variant that the table
        /// does not mention fails to compile here.
        pub(super) fn bind_surface(kind: ActionKind) -> BindSurface {
            match kind {
                $( ActionKind::$SimpleVariant => BindSurface::Simple(stringify!($simple_field)), )*
                $( ActionKind::$ParamVariant => BindSurface::Parametric(stringify!($param_field)), )*
                $( ActionKind::$UnboundVariant => BindSurface::Unbindable, )*
            }
        }

        /// The non-Action top-level keys — style and the id-keyed
        /// binding maps. The Action-derived half of the recognized
        /// set comes from [`bind_surface`].
        const EXTRA_KEYS: &[&str] = &[ $( stringify!($extra_field) ),* ];

        // One compile-time assertion per named-field parametric row:
        // the row's field order against the `Action` declaration's.
        // See `keybind_field_order_check!` for what a failure means.
        $(
            $crate::application::keybinds::surface::keybind_field_order_check!(
                $ParamVariant $param_payload
            );
        )*
    };
}

pub(super) use keybind_arg_names;
pub(super) use keybind_build_action;
pub(super) use keybind_field_order_check;
pub(super) use keybind_surface;
