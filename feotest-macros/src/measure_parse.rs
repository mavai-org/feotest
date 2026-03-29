//! Attribute parsing for `#[measure_experiment(...)]`.

use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, LitStr, Token};

use crate::parse::{parse_duration_str, parse_pacing_str};

/// Parsed attributes from `#[measure_experiment(...)]`.
#[derive(Debug)]
pub struct MeasureAttrs {
    pub use_case: Option<String>,
    pub samples: Option<u32>,
    pub inputs: Option<Vec<String>>,
    pub spec_dir: Option<String>,
    pub experiment_id: Option<String>,
    pub warmup: Option<u32>,
    pub time_budget: Option<String>,
    pub token_budget: Option<u64>,
    pub pacing: Option<String>,
}

impl MeasureAttrs {
    /// Validates required attributes, returning compile errors for missing ones.
    pub fn validate(&self) -> syn::Result<()> {
        if self.use_case.is_none() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "measure_experiment requires `use_case = \"...\"`",
            ));
        }
        if self.samples.is_none() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "measure_experiment requires `samples = N`",
            ));
        }
        Ok(())
    }
}

impl Parse for MeasureAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut attrs = Self {
            use_case: None,
            samples: None,
            inputs: None,
            spec_dir: None,
            experiment_id: None,
            warmup: None,
            time_budget: None,
            token_budget: None,
            pacing: None,
        };

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let _eq: Token![=] = input.parse()?;

            match key.to_string().as_str() {
                "use_case" => {
                    let lit: LitStr = input.parse()?;
                    attrs.use_case = Some(lit.value());
                }
                "samples" => {
                    let lit: LitInt = input.parse()?;
                    let val: u32 = lit.base10_parse()?;
                    if val == 0 {
                        return Err(syn::Error::new(lit.span(), "samples must be >= 1"));
                    }
                    attrs.samples = Some(val);
                }
                "inputs" => {
                    let content;
                    syn::bracketed!(content in input);
                    let mut values = Vec::new();
                    while !content.is_empty() {
                        let lit: LitStr = content.parse()?;
                        values.push(lit.value());
                        if content.peek(Token![,]) {
                            let _comma: Token![,] = content.parse()?;
                        }
                    }
                    if values.is_empty() {
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            "inputs must contain at least one value",
                        ));
                    }
                    attrs.inputs = Some(values);
                }
                "spec_dir" => {
                    let lit: LitStr = input.parse()?;
                    attrs.spec_dir = Some(lit.value());
                }
                "experiment_id" => {
                    let lit: LitStr = input.parse()?;
                    attrs.experiment_id = Some(lit.value());
                }
                "warmup" => {
                    let lit: LitInt = input.parse()?;
                    attrs.warmup = Some(lit.base10_parse()?);
                }
                "time_budget" => {
                    let lit: LitStr = input.parse()?;
                    let val = lit.value();
                    parse_duration_str(&val).map_err(|msg| syn::Error::new(lit.span(), msg))?;
                    attrs.time_budget = Some(val);
                }
                "token_budget" => {
                    let lit: LitInt = input.parse()?;
                    attrs.token_budget = Some(lit.base10_parse()?);
                }
                "pacing" => {
                    let lit: LitStr = input.parse()?;
                    let val = lit.value();
                    parse_pacing_str(&val).map_err(|msg| syn::Error::new(lit.span(), msg))?;
                    attrs.pacing = Some(val);
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown attribute: `{other}`"),
                    ));
                }
            }

            if input.peek(Token![,]) {
                let _comma: Token![,] = input.parse()?;
            }
        }

        Ok(attrs)
    }
}
