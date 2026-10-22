// SPDX-License-Identifier: MPL-2.0

//! Custom derive macros for the Mandala application crate. Two
//! derives today, both applied to `Action` and both existing to make
//! a hand-sync into a build error: [`ActionClassify`], which reads
//! each variant's classification attributes, and
//! [`PayloadFieldNames`], which reads each variant's *field names*
//! off the declaration so a second declaration of them elsewhere can
//! be checked against it at compile time.
//!
//! ## `#[derive(ActionClassify)]`
//!
//! Declarative replacement for the three classifier methods that
//! used to live as 113-arm hand-written matches in
//! `keybinds/action/{destructive, context, wasm}.rs`:
//!
//! - `is_destructive(&self) -> bool` — the privilege gate consulted
//!   by `SourceTier::allows_action`.
//! - `context(&self) -> InputContext` — modal-context routing for
//!   the keybind resolver.
//! - `wasm_compatibility(&self) -> WasmCompatibility` — WASM-port
//!   classification consulted by the cross-platform dispatcher.
//!
//! The derive emits matching `pub fn`s on **both** the source enum
//! (e.g. `Action`) and its strum-derived discriminant (e.g.
//! `ActionKind`). The `ActionKind` versions take `self` (no payload
//! destructuring); the `Action` versions are thin delegates that
//! forward to `ActionKind::from(self).method()`. Callers reach for
//! whichever shape is closer to hand.
//!
//! The discriminant name is **auto-detected** from the
//! `#[strum_discriminants(name(...))]` attribute on the input enum.
//! Adding `ActionClassify` without that attribute is a compile
//! error.
//!
//! ## Per-variant attribute syntax
//!
//! ```ignore
//! #[derive(EnumDiscriminants, ActionClassify)]
//! #[strum_discriminants(name(ActionKind))]
//! pub enum Action {
//!     #[action(context = Document, wasm = Compatible)]
//!     Undo,
//!
//!     #[action(context = Document, wasm = Compatible, destructive)]
//!     DeleteSelection,
//!
//!     #[action(context = Document, wasm = NativeOnly, destructive)]
//!     OpenDocument(String),
//!
//!     #[action(context = Console, wasm = NativeOnly)]
//!     ConsoleSubmit,
//! }
//! ```
//!
//! - `context = <ident>` — required. Becomes `InputContext::<ident>`.
//! - `wasm = <ident>` — required. Becomes `WasmCompatibility::<ident>`.
//! - `destructive` — bare flag. Absent ⇒ `false`. `destructive = true`
//!   is rejected (no other key is bare-flag-shaped; mixing
//!   conventions is the kind of "good enough" we don't accept).
//!
//! ## Forcing function
//!
//! Three compile-time guards land on a contributor adding a new
//! variant without classifying it:
//!
//! 1. Missing `#[action(...)]` — `compile_error!` cites the variant
//!    name, points at the variant declaration.
//! 2. Missing `context` or `wasm` key inside `#[action(...)]` —
//!    `compile_error!` points at the attribute.
//! 3. The generated matches are themselves exhaustive over the
//!    discriminant enum — Rust's match-exhaustiveness check is the
//!    last line of defence.
//!
//! All three preserve the privilege-gate contract previously
//! enforced by hand-written exhaustive matches.
//!
//! ## `#[derive(PayloadFieldNames)]`
//!
//! Publishes each named-field variant's field names, in declaration
//! order, as a `pub const` named after the variant inside a
//! generated `<enum_in_snake_case>_payload_fields` module:
//!
//! ```ignore
//! #[derive(PayloadFieldNames)]
//! pub enum Action {
//!     AddSection { at: String, text: String },
//!     SetEdgeBodyGlyph(String),   // tuple variants emit nothing
//! }
//!
//! // generated:
//! pub mod action_payload_fields {
//!     pub const AddSection: &[&str] = &["at", "text"];
//! }
//! ```
//!
//! ### Why a *second* statement of the field names is the point
//!
//! The consts exist to be compared against a field-name list written
//! somewhere else. In Mandala that somewhere else is the
//! `keybind_surface!` table, whose row for a payload-carrying
//! `Action` names the same fields to define the positional `args`
//! contract of a `keybinds.json` binding. A struct expression is
//! order-free, so a row whose fields are written in the wrong order
//! compiles, resolves, and hands the user's first argument to the
//! second field — `AddSection { at, text }` written `{ text, at }`
//! passed the entire test suite.
//!
//! With this derive, the table's row and the enum declaration are two
//! independent sources for the same list, and a `const fn` comparison
//! between them under `const _: () = assert!(…)` makes the
//! transposition an `error[E0080]` at the offending row. That is what
//! a mirror of the declaration could not do: a mirror agrees with
//! itself.
//!
//! Tuple variants are skipped because the defect cannot exist there —
//! their "field names" at the other site are local bindings whose
//! order is the same repetition in the pattern and the constructor.

// CODE_CONVENTIONS §9 closes with "Bare `unwrap()` outside tests is
// a bug", and this is the half of that rule an editor can tell you
// about while you type. `util::unwrap_posture` is the other half —
// it reads the workspace's source text and fails every `./test.sh`
// run; the lint fails only `./test.sh --lint`, a hard gate since
// clippy's baseline reached zero, but one you have to ask for. Two
// mechanisms rather than one because they disagree usefully: the lint sees
// post-expansion code the text scan cannot read, and the scan sees
// the `pub mod tests;` trees the lint has to be told about.
//
// The `cfg_attr` is what keeps the lint off test code. A
// `#[cfg(test)] mod` does not exist in the build where the lint is
// live, and in the build where it does exist the whole crate is
// allowed — so `unwrap()` stays the right spelling in a test and a
// bug everywhere else.
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Ident, Meta, Token, Variant};

#[cfg(test)]
use syn::parse_quote;

#[proc_macro_derive(ActionClassify, attributes(action))]
pub fn derive_action_classify(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_action_classify_impl(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Pure entry point: takes a parsed `DeriveInput`, returns either
/// the generated impl or a `syn::Error`. Split from the
/// `proc_macro::TokenStream` shim above so the body can be unit-
/// tested with `parse_quote!` without standing up a proc-macro
/// invocation harness.
fn derive_action_classify_impl(input: DeriveInput) -> syn::Result<TokenStream2> {
    let action_name = &input.ident;
    let discriminant = discriminant_name(&input)?;

    let Data::Enum(data_enum) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "ActionClassify can only be derived on enums",
        ));
    };

    let mut errors: Option<syn::Error> = None;
    let mut destructive_arms = Vec::with_capacity(data_enum.variants.len());
    let mut context_arms = Vec::with_capacity(data_enum.variants.len());
    let mut wasm_arms = Vec::with_capacity(data_enum.variants.len());

    for variant in &data_enum.variants {
        let variant_name = &variant.ident;
        match parse_action_attrs(variant) {
            Ok(ActionAttrs {
                context,
                wasm,
                destructive,
            }) => {
                destructive_arms.push(quote! {
                    #discriminant::#variant_name => #destructive
                });
                context_arms.push(quote! {
                    #discriminant::#variant_name => InputContext::#context
                });
                wasm_arms.push(quote! {
                    #discriminant::#variant_name => WasmCompatibility::#wasm
                });
            }
            Err(e) => match &mut errors {
                Some(acc) => acc.combine(e),
                None => errors = Some(e),
            },
        }
    }

    if let Some(e) = errors {
        return Err(e);
    }

    Ok(quote! {
        impl #discriminant {
            /// Whether the Action this kind represents mutates
            /// persistent state (filesystem, document model
            /// bypassing the undo stack, clipboard) or reaches an
            /// editor modal that mutates on commit. Generated from
            /// the `destructive` flag on each variant's
            /// `#[action(...)]` attribute by `mandala_derive`.
            pub fn is_destructive(self) -> bool {
                match self {
                    #( #destructive_arms ),*
                }
            }

            /// The input context this Action belongs to. Generated
            /// from the `context = ...` key on each variant's
            /// `#[action(...)]` attribute.
            pub fn context(self) -> InputContext {
                match self {
                    #( #context_arms ),*
                }
            }

            /// Whether this Action can fire on WASM today. See
            /// `WASM_CONVERGENCE.md` for the porting path each
            /// `NativeOnly` arm follows. Generated from the
            /// `wasm = ...` key on each variant's `#[action(...)]`
            /// attribute.
            pub fn wasm_compatibility(self) -> WasmCompatibility {
                match self {
                    #( #wasm_arms ),*
                }
            }
        }

        impl #action_name {
            /// See [`#discriminant::is_destructive`]. Thin delegate
            /// that converts to the discriminant kind first so the
            /// classification body need not destructure payloads.
            pub fn is_destructive(&self) -> bool {
                #discriminant::from(self).is_destructive()
            }

            /// See [`#discriminant::context`]. Thin delegate.
            pub fn context(&self) -> InputContext {
                #discriminant::from(self).context()
            }

            /// See [`#discriminant::wasm_compatibility`]. Thin
            /// delegate.
            pub fn wasm_compatibility(&self) -> WasmCompatibility {
                #discriminant::from(self).wasm_compatibility()
            }
        }
    })
}

struct ActionAttrs {
    context: Ident,
    wasm: Ident,
    destructive: bool,
}

fn parse_action_attrs(variant: &Variant) -> syn::Result<ActionAttrs> {
    let mut iter = variant.attrs.iter().filter(|a| a.path().is_ident("action"));
    let action_attr = iter.next().ok_or_else(|| {
        syn::Error::new_spanned(
            &variant.ident,
            format!(
                "variant `{}` is missing #[action(context = ..., wasm = ...)] attribute — \
                 ActionClassify requires every variant to declare its classification",
                variant.ident,
            ),
        )
    })?;
    if let Some(extra) = iter.next() {
        return Err(syn::Error::new_spanned(
            extra,
            format!(
                "variant `{}` has multiple #[action(...)] attributes; merge them",
                variant.ident,
            ),
        ));
    }

    let mut context: Option<Ident> = None;
    let mut wasm: Option<Ident> = None;
    let mut destructive = false;

    action_attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("context") {
            context = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("wasm") {
            wasm = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("destructive") {
            // Reject `destructive = ...`. The bare-flag form is
            // documented; a `destructive = true` typo would otherwise
            // be silently accepted (the rhs is consumed by the
            // default value parser).
            if meta.input.peek(syn::Token![=]) {
                return Err(meta.error(
                    "`destructive` is a bare flag; remove the `= …` (`destructive`, not `destructive = true`)",
                ));
            }
            destructive = true;
        } else {
            return Err(meta.error(
                "unknown #[action(...)] key; expected `context = <ident>`, \
                 `wasm = <ident>`, or `destructive`",
            ));
        }
        Ok(())
    })?;

    let context = context.ok_or_else(|| {
        syn::Error::new_spanned(
            action_attr,
            format!(
                "variant `{}` missing `context = <ident>` in #[action(...)]",
                variant.ident,
            ),
        )
    })?;
    let wasm = wasm.ok_or_else(|| {
        syn::Error::new_spanned(
            action_attr,
            format!(
                "variant `{}` missing `wasm = <ident>` in #[action(...)]",
                variant.ident,
            ),
        )
    })?;

    Ok(ActionAttrs {
        context,
        wasm,
        destructive,
    })
}

/// Pull the discriminant enum's name out of `#[strum_discriminants(
/// name(<ident>))]`. The derive intentionally couples to strum's
/// `EnumDiscriminants` rather than declaring its own discriminant
/// shape — generating the discriminant ourselves would be a parallel
/// path to one strum already provides; consuming strum's output
/// keeps the seam single.
///
/// Parses the `strum_discriminants` attribute as a comma-separated
/// list of `Meta` items (vs. `parse_nested_meta`, which is awkward
/// when neighbouring keys also use the `key(...)` shape — strum's
/// `derive(Hash, EnumIter)` would need its own consume step).
fn discriminant_name(input: &DeriveInput) -> syn::Result<Ident> {
    let strum_attr = input
        .attrs
        .iter()
        .find(|a| a.path().is_ident("strum_discriminants"))
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &input.ident,
                "ActionClassify requires `#[strum_discriminants(name(...))]` on the same enum \
                 so the generated impls land on the discriminant kind. Add the strum attribute, \
                 or remove the ActionClassify derive.",
            )
        })?;

    let metas: Punctuated<Meta, Token![,]> = strum_attr.parse_args_with(Punctuated::parse_terminated)?;
    for meta in &metas {
        if let Meta::List(ml) = meta {
            if ml.path.is_ident("name") {
                return syn::parse2::<Ident>(ml.tokens.clone());
            }
        }
    }
    Err(syn::Error::new_spanned(
        strum_attr,
        "`#[strum_discriminants(...)]` is missing `name(<ident>)`; ActionClassify needs to \
         know the discriminant enum's name",
    ))
}

#[proc_macro_derive(PayloadFieldNames)]
pub fn derive_payload_field_names(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_payload_field_names_impl(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Pure entry point, split from the `proc_macro::TokenStream` shim
/// for the same reason [`derive_action_classify_impl`] is: the body
/// is then reachable from a unit test through `parse_quote!`.
fn derive_payload_field_names_impl(input: DeriveInput) -> syn::Result<TokenStream2> {
    let enum_name = &input.ident;
    let Data::Enum(data_enum) = &input.data else {
        return Err(syn::Error::new_spanned(
            enum_name,
            "PayloadFieldNames can only be derived on enums",
        ));
    };

    // Named after the enum so two enums in one module can both carry
    // the derive. The module is the namespace that lets the const be
    // named for the variant verbatim — which is what makes the const
    // reachable from a `macro_rules!` expansion, where an `ident`
    // capture can be pasted but not case-converted.
    let module = Ident::new(
        &format!("{}_payload_fields", snake_case(&enum_name.to_string())),
        enum_name.span(),
    );

    let consts: Vec<TokenStream2> = data_enum
        .variants
        .iter()
        .filter_map(|variant| match &variant.fields {
            Fields::Named(named) => Some((&variant.ident, named)),
            _ => None,
        })
        .map(|(variant_name, named)| {
            let names = named.named.iter().map(|field| {
                field
                    .ident
                    .as_ref()
                    .expect("syn guarantees an ident for every field of a Fields::Named variant")
                    .to_string()
            });
            let doc = format!(
                "The field names of `{enum_name}::{variant_name}`, in the order the variant \
                 declares them.",
            );
            quote! {
                #[doc = #doc]
                pub const #variant_name: &[&str] = &[ #(#names),* ];
            }
        })
        .collect();

    let module_doc = format!(
        "Field names of every named-field variant of `{enum_name}`, read off the declaration by \
         `mandala_derive::PayloadFieldNames`. Exists so a second, independent statement of a \
         variant's field order can be checked against the declaration at compile time; see the \
         derive's documentation for the defect that motivates it.",
    );
    Ok(quote! {
        #[doc = #module_doc]
        // The consts are named for their variants, which is the
        // whole mechanism — a `macro_rules!` caller pastes the
        // variant ident straight into the path. `dead_code` because
        // an enum may well have named-field variants that no second
        // site restates.
        #[allow(non_upper_case_globals, dead_code)]
        pub mod #module {
            #( #consts )*
        }
    })
}

/// `ActionKind` → `action_kind`. Only ever applied to a type ident,
/// so the acronym cases stock conversions disagree about (`HTTPBody`)
/// are outside its remit; it inserts an underscore before every
/// uppercase char but the first and lowercases the rest.
fn snake_case(ident: &str) -> String {
    let mut out = String::with_capacity(ident.len() + 4);
    for (i, ch) in ident.char_indices() {
        if ch.is_uppercase() && i != 0 {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    //! Direct coverage of the parser and discriminant-name lookup.
    //! The proc-macro entry point itself takes a `proc_macro::TokenStream`
    //! that isn't constructible from a unit test, so the tests target
    //! [`derive_action_classify_impl`] (pure `DeriveInput` →
    //! `TokenStream2`) and the parser helpers.
    use super::*;

    #[test]
    fn missing_action_attribute_errors_with_variant_name() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            enum Foo {
                Bare,
            }
        };
        let err = derive_action_classify_impl(input).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Bare"), "error must cite variant name: {msg}");
        assert!(
            msg.contains("missing #[action("),
            "error must name the missing attribute: {msg}",
        );
    }

    #[test]
    fn missing_context_key_errors() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            enum Foo {
                #[action(wasm = Compatible)]
                X,
            }
        };
        let err = derive_action_classify_impl(input).unwrap_err();
        assert!(err.to_string().contains("missing `context"));
    }

    #[test]
    fn missing_wasm_key_errors() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            enum Foo {
                #[action(context = Document)]
                X,
            }
        };
        let err = derive_action_classify_impl(input).unwrap_err();
        assert!(err.to_string().contains("missing `wasm"));
    }

    #[test]
    fn unknown_key_errors() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            enum Foo {
                #[action(context = Document, wasm = Compatible, banana = 3)]
                X,
            }
        };
        let err = derive_action_classify_impl(input).unwrap_err();
        assert!(err.to_string().contains("unknown #[action"));
    }

    #[test]
    fn destructive_with_value_rejected() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            enum Foo {
                #[action(context = Document, wasm = Compatible, destructive = true)]
                X,
            }
        };
        let err = derive_action_classify_impl(input).unwrap_err();
        assert!(
            err.to_string().contains("bare flag"),
            "expected the `destructive = …` rejection: {err}",
        );
    }

    #[test]
    fn duplicate_action_attribute_rejected() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            enum Foo {
                #[action(context = Document, wasm = Compatible)]
                #[action(destructive)]
                X,
            }
        };
        let err = derive_action_classify_impl(input).unwrap_err();
        assert!(err.to_string().contains("multiple #[action"));
    }

    #[test]
    fn missing_strum_discriminants_errors() {
        let input: DeriveInput = parse_quote! {
            enum Foo {
                #[action(context = Document, wasm = Compatible)]
                X,
            }
        };
        let err = derive_action_classify_impl(input).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("strum_discriminants"),
            "error must point at the missing strum attribute: {msg}",
        );
    }

    #[test]
    fn non_enum_input_errors() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            struct Foo;
        };
        let err = derive_action_classify_impl(input).unwrap_err();
        assert!(err.to_string().contains("only be derived on enums"));
    }

    #[test]
    fn errors_accumulate_across_variants() {
        // `syn::Error::combine` chains multiple errors; `to_string`
        // only shows the head, but `into_iter` walks all of them
        // and `to_compile_error` lowers each as a separate
        // `compile_error!` token. Iterate explicitly so the test
        // pins "every bad variant got a diagnostic," which is the
        // user-facing contract.
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            enum Foo {
                #[action(context = Document)]
                MissingWasm,
                #[action(wasm = Compatible)]
                MissingContext,
            }
        };
        let err = derive_action_classify_impl(input).unwrap_err();
        let messages: Vec<String> = err.into_iter().map(|e| e.to_string()).collect();
        assert!(
            messages.iter().any(|m| m.contains("MissingWasm")),
            "expected diagnostic for MissingWasm in: {messages:?}",
        );
        assert!(
            messages.iter().any(|m| m.contains("MissingContext")),
            "expected diagnostic for MissingContext in: {messages:?}",
        );
    }

    #[test]
    fn discriminant_name_auto_detected() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(WeirdName))]
            enum Foo {
                #[action(context = Document, wasm = Compatible)]
                X,
            }
        };
        let out = derive_action_classify_impl(input).unwrap();
        let s = out.to_string();
        assert!(s.contains("impl WeirdName"), "discriminant respected: {s}");
        assert!(
            s.contains("WeirdName :: from"),
            "delegates use detected name: {s}"
        );
    }

    #[test]
    fn destructive_flag_emits_true() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            enum Foo {
                #[action(context = Document, wasm = NativeOnly, destructive)]
                X,
            }
        };
        let out = derive_action_classify_impl(input).unwrap();
        let s = out.to_string();
        assert!(s.contains("FooKind :: X => true"), "is_destructive true arm: {s}");
        assert!(s.contains("InputContext :: Document"), "context arm: {s}");
        assert!(s.contains("WasmCompatibility :: NativeOnly"), "wasm arm: {s}");
    }

    #[test]
    fn omitted_destructive_emits_false() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            enum Foo {
                #[action(context = Document, wasm = Compatible)]
                X,
            }
        };
        let out = derive_action_classify_impl(input).unwrap();
        assert!(out.to_string().contains("FooKind :: X => false"));
    }

    #[test]
    fn delegate_methods_emitted_on_source_enum() {
        let input: DeriveInput = parse_quote! {
            #[strum_discriminants(name(FooKind))]
            enum Foo {
                #[action(context = Document, wasm = Compatible)]
                X,
            }
        };
        let out = derive_action_classify_impl(input).unwrap();
        let s = out.to_string();
        // The delegate impl block on Foo emits all three methods,
        // each forwarding via `FooKind::from(self).method()`.
        assert!(s.contains("impl Foo"), "delegate impl block: {s}");
        for method in ["is_destructive", "context", "wasm_compatibility"] {
            assert!(
                s.contains(&format!("fn {method}")),
                "delegate `{method}` emitted: {s}",
            );
        }
    }

    // ── PayloadFieldNames ────────────────────────────────────────

    #[test]
    fn payload_field_names_emits_declaration_order() {
        // The property the whole derive exists for: the order in the
        // emitted const is the order of the *declaration*, so a
        // second statement of it elsewhere can be checked against it.
        let input: DeriveInput = parse_quote! {
            enum Action {
                AddSection { at: String, text: String },
            }
        };
        let out = derive_payload_field_names_impl(input).unwrap();
        let s = out.to_string();
        assert!(
            s.contains(r#"pub const AddSection : & [& str] = & ["at" , "text"]"#),
            "declared order must survive verbatim: {s}",
        );
    }

    #[test]
    fn payload_field_names_transposed_declaration_emits_transposed_const() {
        // The negative of the test above, and the reason the check
        // downstream is not a mirror: the const tracks the
        // declaration rather than a fixed expectation, so the two
        // sources can disagree.
        let input: DeriveInput = parse_quote! {
            enum Action {
                AddSection { text: String, at: String },
            }
        };
        let s = derive_payload_field_names_impl(input).unwrap().to_string();
        assert!(
            s.contains(r#"& ["text" , "at"]"#),
            "the const must follow the declaration, not a canonical order: {s}",
        );
    }

    #[test]
    fn payload_field_names_skips_tuple_and_unit_variants() {
        let input: DeriveInput = parse_quote! {
            enum Action {
                Undo,
                SetEdgeBodyGlyph(String),
                SetColor { axis: ColorAxis, value: String },
            }
        };
        let s = derive_payload_field_names_impl(input).unwrap().to_string();
        assert!(
            s.contains("pub const SetColor"),
            "named-field variant emitted: {s}"
        );
        assert!(
            !s.contains("SetEdgeBodyGlyph"),
            "a tuple variant has no declared field names to publish: {s}",
        );
        assert!(!s.contains("Undo"), "a unit variant has no fields at all: {s}");
    }

    #[test]
    fn payload_field_names_module_is_named_for_the_enum() {
        // Two enums in one module both carrying the derive must not
        // collide, so the module name is derived rather than fixed.
        let input: DeriveInput = parse_quote! {
            enum ActionKind {
                X { a: u8 },
            }
        };
        let s = derive_payload_field_names_impl(input).unwrap().to_string();
        assert!(
            s.contains("pub mod action_kind_payload_fields"),
            "module named for the enum: {s}",
        );
    }

    #[test]
    fn payload_field_names_non_enum_input_errors() {
        let input: DeriveInput = parse_quote! {
            struct Action { at: String }
        };
        let err = derive_payload_field_names_impl(input).unwrap_err();
        assert!(err.to_string().contains("only be derived on enums"));
    }

    #[test]
    fn snake_case_converts_type_idents() {
        assert_eq!(snake_case("Action"), "action");
        assert_eq!(snake_case("ActionKind"), "action_kind");
        assert_eq!(snake_case("X"), "x");
    }
}
