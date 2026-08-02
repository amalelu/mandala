// SPDX-License-Identifier: MPL-2.0

//! Custom derive macros for the Mandala application crate. Today
//! provides exactly one derive: [`ActionClassify`].
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

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Data, DeriveInput, Ident, Meta, Token, Variant};

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
}
