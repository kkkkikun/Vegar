#![cfg_attr(feature = "nightly", feature(proc_macro_expand))]
#![doc = include_str!("../README.md")]

use proc_macro::{LexError, TokenStream};
use quote::{quote, ToTokens};
use syn::parse_macro_input;
use syn::{Error, LitStr};

use axconfig_gen::{Config, OutputFormat};

fn compiler_error<T: ToTokens>(tokens: T, msg: String) -> TokenStream {
    Error::new_spanned(tokens, msg).to_compile_error().into()
}

/// Parses TOML config content and expands it into Rust code.
///
/// # Example
///
/// See the [crate-level documentation][crate].
#[proc_macro]
pub fn parse_configs(config_toml: TokenStream) -> TokenStream {
    #[cfg(feature = "nightly")]
    let config_toml = match config_toml.expand_expr() {
        Ok(s) => s,
        Err(e) => {
            return Error::new(proc_macro2::Span::call_site(), e.to_string())
                .to_compile_error()
                .into()
        }
    };

    let config_toml = parse_macro_input!(config_toml as LitStr).value();
    let code = Config::from_toml(&config_toml).and_then(|cfg| cfg.dump(OutputFormat::Rust));
    match code {
        Ok(code) => code
            .parse()
            .unwrap_or_else(|e: LexError| compiler_error(config_toml, e.to_string())),
        Err(e) => compiler_error(config_toml, e.to_string()),
    }
}

/// Includes a TOML format config file and expands it into Rust code.
///
/// The given path should be an absolute path or a path relative to your
/// project's `Cargo.toml`.
///
/// # Example
///
/// See the [crate-level documentation][crate].
#[proc_macro]
pub fn include_configs(path: TokenStream) -> TokenStream {
    #[cfg(feature = "nightly")]
    let path = match path.expand_expr() {
        Ok(s) => s,
        Err(e) => {
            return Error::new(proc_macro2::Span::call_site(), e.to_string())
                .to_compile_error()
                .into()
        }
    };

    let path = parse_macro_input!(path as LitStr);
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let cfg_path = std::path::Path::new(&root).join(path.value());

    let Ok(config_toml) = std::fs::read_to_string(&cfg_path) else {
        return compiler_error(path, format!("Failed to read config file: {:?}", cfg_path));
    };

    quote! {
        ::axconfig_gen_macros::parse_configs!(#config_toml);
    }
    .into()
}
