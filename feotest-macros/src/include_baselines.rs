//! Code expansion for `include_baselines!`.
//!
//! Reads a directory at macro expansion time and bakes the YAML files it
//! finds into the binary as [`EmbeddedBaseline`] inventory submissions.
//!
//! # Directory convention
//!
//! ```text
//! baselines/
//! ├── <spec-name>/
//! │   ├── <method-name>.yaml
//! │   └── ...
//! └── <another-spec>/
//!     └── ...
//! ```
//!
//! Each YAML file is embedded verbatim via `include_str!`; its filename
//! (without `.yaml`) becomes `method_name` and its parent directory name
//! becomes `spec_name`.

use std::path::{Path, PathBuf};

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Error, LitStr, Result, parse2};

pub fn expand(input: TokenStream) -> Result<TokenStream> {
    let path_lit: LitStr = parse2(input)?;
    let input_span = path_lit.span();
    let relative = PathBuf::from(path_lit.value());
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| {
        Error::new(
            input_span,
            "CARGO_MANIFEST_DIR is unset; include_baselines! must be invoked from a cargo build",
        )
    })?;
    let baseline_root = PathBuf::from(manifest_dir).join(&relative);

    let mut entries: Vec<Entry> = Vec::new();
    collect_entries(&baseline_root, &mut entries, input_span)?;

    let tokens = entries.iter().enumerate().map(|(idx, entry)| {
        let ident = format_ident!("__feotest_embedded_baseline_{idx}");
        let spec_name = &entry.spec_name;
        let method_name = &entry.method_name;
        let path = entry.path.to_string_lossy().into_owned();
        quote! {
            #[doc(hidden)]
            #[allow(non_snake_case, reason = "generated embedding module")]
            mod #ident {
                ::feotest::inventory::submit! {
                    ::feotest::sentinel::EmbeddedBaseline {
                        spec_name: #spec_name,
                        method_name: #method_name,
                        yaml: ::core::include_str!(#path),
                    }
                }
            }
        }
    });

    Ok(quote! {
        #(#tokens)*
    })
}

struct Entry {
    spec_name: String,
    method_name: String,
    path: PathBuf,
}

fn collect_entries(root: &Path, out: &mut Vec<Entry>, span: proc_macro2::Span) -> Result<()> {
    let spec_dirs = std::fs::read_dir(root).map_err(|e| {
        Error::new(
            span,
            format!("cannot read baseline directory {}: {e}", root.display()),
        )
    })?;
    for spec_entry in spec_dirs.flatten() {
        let spec_path = spec_entry.path();
        if !spec_path.is_dir() {
            continue;
        }
        let Some(spec_name) = spec_path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let method_dirs = std::fs::read_dir(&spec_path).map_err(|e| {
            Error::new(
                span,
                format!("cannot read spec directory {}: {e}", spec_path.display()),
            )
        })?;
        for method_entry in method_dirs.flatten() {
            let path = method_entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
                continue;
            }
            let Some(method_name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            out.push(Entry {
                spec_name: spec_name.to_owned(),
                method_name: method_name.to_owned(),
                path: path.clone(),
            });
        }
    }
    Ok(())
}
