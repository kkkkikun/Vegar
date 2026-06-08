#![doc = include_str!("../README.md")]

use convert_case::Casing;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{Ident, ItemImpl, ItemTrait, parse_macro_input, spanned::Spanned};

macro_rules! bail {
    ($i:expr, $msg:expr) => {
        return syn::parse::Error::new($i, $msg).to_compile_error().into();
    };
}

fn get_crate_name() -> String {
    std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "unknown".to_string())
}

fn get_crate_version() -> String {
    std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".to_string())
}

fn prefix_version() -> String {
    let version = lenient_semver::parse(&get_crate_version()).unwrap();
    let major = version.major;
    let minor = version.minor;
    if major == 0 {
        format!("0_{minor}")
    } else {
        major.to_string()
    }
}

fn extern_fn_name(crate_name: &str, fn_name: &Ident) -> Ident {
    let crate_name = crate_name.to_lowercase().replace("-", "_");
    // let version = prefix_version();

    format_ident!("__{crate_name}_{fn_name}")
}

fn parse_def_extern_trait_args(
    args: TokenStream,
) -> Result<(String, bool, Option<String>), String> {
    if args.is_empty() {
        return Ok(("rust".to_string(), false, None)); // 默认使用 Rust ABI，默认生成 impl_trait! 宏，无自定义模块路径
    }

    let args_str = args.to_string();
    let mut abi = None;
    let mut not_def_impl = false;
    let mut mod_path = None;

    // 简单解析 abi="value"、not_def_impl 和 mod_path="value" 形式
    let parts: Vec<&str> = args_str.split(',').collect();

    for part in parts {
        let part = part.trim();
        if part.starts_with("abi")
            && let Some(start) = part.find('"')
            && let Some(end) = part.rfind('"')
            && start < end
        {
            abi = Some(part[start + 1..end].to_string());
        } else if part.starts_with("mod_path")
            && let Some(start) = part.find('"')
            && let Some(end) = part.rfind('"')
            && start < end
        {
            mod_path = Some(part[start + 1..end].to_string());
        } else if part == "not_def_impl" {
            not_def_impl = true;
        }
    }

    let abi = abi.unwrap_or_else(|| "rust".to_string());

    if abi != "c" && abi != "rust" {
        return Err("Invalid abi parameter. Supported values: \"c\", \"rust\"".to_string());
    }

    Ok((abi, not_def_impl, mod_path))
}

/// Defines an extern trait that can be called across FFI boundaries.
///
/// This macro converts a regular Rust trait into a trait that can be called through FFI.
/// It generates:
/// 1. The original trait definition
/// 2. A module containing wrapper functions that call external implementations
/// 3. Optionally, a helper macro `impl_trait!` for implementing the trait (unless `not_def_impl` is specified)
/// 4. A checker function to ensure the trait is properly implemented
///
/// # Arguments
/// - `abi`: Optional parameter specifying ABI type ("c" or "rust"), defaults to "rust"
/// - `not_def_impl`: Optional parameter to skip generating the `impl_trait!` macro
///
/// # Example
/// ```rust
/// #[def_extern_trait(abi = "c")]
/// trait Calculator {
///     fn add(&self, a: i32, b: i32) -> i32;
///     fn multiply(&self, a: i32, b: i32) -> i32;
/// }
///
/// // Skip generating impl_trait! macro
/// #[def_extern_trait(abi = "c", not_def_impl)]
/// trait Calculator2 {
///     fn add(&self, a: i32, b: i32) -> i32;
/// }
/// ```
///
/// This will generate a `calculator` module containing functions that can call external implementations.
#[proc_macro_attribute]
pub fn def_extern_trait(args: TokenStream, input: TokenStream) -> TokenStream {
    let (abi, not_def_impl, _mod_path) = match parse_def_extern_trait_args(args) {
        Ok((abi, not_def_impl, mod_path)) => (abi, not_def_impl, mod_path),
        Err(error_msg) => {
            bail!(Span::call_site(), error_msg);
        }
    };

    let input = parse_macro_input!(input as ItemTrait);
    let vis = input.vis.clone();
    let mod_name = format_ident!(
        "{}",
        input.ident.to_string().to_case(convert_case::Case::Snake)
    );
    let crate_name_str = get_crate_name();

    let mut fn_list = vec![];
    let crate_name = format_ident!("{}", crate_name_str.replace("-", "_"));
    let mut crate_path_tokens = quote! { #crate_name };
    if let Some(mod_path) = _mod_path {
        // 解析 mod_path 并生成路径tokens
        let path_segments: Vec<&str> = mod_path.split("::").collect();
        let path_idents: Vec<proc_macro2::Ident> = path_segments
            .iter()
            .map(|segment| format_ident!("{}", segment))
            .collect();
        crate_path_tokens = quote! { #crate_name::#(#path_idents)::* };
    }

    let crate_name_version = format!("{}_{}", crate_name_str, prefix_version());

    for item in &input.items {
        if let syn::TraitItem::Fn(func) = item {
            let fn_name = func.sig.ident.clone();
            let extern_fn_name = extern_fn_name(&crate_name_version, &fn_name);

            let attrs = &func.attrs;
            let inputs = &func.sig.inputs;
            let output = &func.sig.output;
            let generics = &func.sig.generics;
            let unsafety = &func.sig.unsafety;

            let mut param_names = vec![];
            let mut param_types = vec![];

            for input in inputs {
                if let syn::FnArg::Typed(pat_type) = input {
                    param_names.push(&pat_type.pat);
                    param_types.push(&pat_type.ty);
                }
            }

            let extern_abi = if abi == "rust" { "Rust" } else { "C" };

            fn_list.push(quote! {
                #(#attrs)*
                pub #unsafety fn #fn_name #generics (#inputs) #output {
                    unsafe extern #extern_abi {
                        fn #extern_fn_name #generics (#inputs) #output;
                    }
                    unsafe{ #extern_fn_name(#(#param_names),*) }
                }
            });
        } else {
            bail!(
                item.span(),
                "Only function items are allowed in extern traits"
            );
        }
    }

    let _warn_fn_name = format_ident!(
        "Trait_{}_in_crate_{}_{}_need_impl",
        input.ident,
        crate_name_str.replace("-", "_"),
        prefix_version()
    );

    let generated_macro = if not_def_impl {
        quote! {}
    } else {
        quote! {
            /// Helper macro to implement the extern trait for a type.
            pub use trait_ffi::impl_extern_trait;

            /// Implement the extern trait for a type.
            #[macro_export]
            macro_rules! impl_trait {
                (impl $trait:ident for $type:ty { $($body:tt)* }) => {
                    #[#crate_path_tokens::impl_extern_trait(name = #crate_name_version, abi = #abi)]
                    impl $trait for $type {
                        $($body)*
                    }

                    // #[allow(snake_case)]
                    // #[unsafe(no_mangle)]
                    // extern "C" fn #warn_fn_name() { }
                };
            }
        }
    };

    quote! {
        #input

        /// Module generated by `trait-ffi`.
        #vis mod #mod_name {
            use super::*;
            /// `trait-ffi` generated.
            // pub fn ____checker_do_not_use(){
            //     unsafe extern "C" {
            //         fn #warn_fn_name();
            //     }
            //     unsafe { #warn_fn_name() };
            // }
            #(#fn_list)*
        }

        #generated_macro
    }
    .into()
}

fn parse_extern_trait_args(args: TokenStream) -> Result<(String, String), String> {
    if args.is_empty() {
        return Err(
            "Missing parameters. Usage: #[impl_extern_trait(name=\"crate_name\", abi=\"c\")]"
                .to_string(),
        );
    }

    let args_str = args.to_string();
    let mut name = None;
    let mut abi = None;

    let parts: Vec<&str> = args_str.split(',').collect();

    for part in parts {
        let part = part.trim();
        if part.starts_with("name") {
            if let Some(start) = part.find('"')
                && let Some(end) = part.rfind('"')
                && start < end
            {
                name = Some(part[start + 1..end].to_string());
            }
        } else if part.starts_with("abi")
            && let Some(start) = part.find('"')
            && let Some(end) = part.rfind('"')
            && start < end
        {
            abi = Some(part[start + 1..end].to_string());
        }
    }

    let name = name.ok_or_else(|| {
        "Missing name parameter. Usage: #[impl_extern_trait(name=\"crate_name\", abi=\"c\")]"
            .to_string()
    })?;
    let abi = abi.unwrap_or_else(|| "c".to_string());

    if abi != "c" && abi != "rust" {
        return Err("Invalid abi parameter. Supported values: \"c\", \"rust\"".to_string());
    }

    Ok((name, abi))
}

/// Implements an extern trait for a type and generates corresponding C function exports.
///
/// This macro takes a trait implementation and generates extern "C" functions that can be
/// called from other languages. Each method in the trait implementation gets a corresponding
/// extern function with a mangled name based on the crate name and version.
///
/// # Arguments
/// - `name`: The name of the crate that defines the extern trait
/// - `abi`: The ABI to use for the extern functions ("c" or "rust"), defaults to "c"
///
/// # Example
/// ```rust
/// struct Calculator;
///
/// #[impl_extern_trait(name = "calculator_crate", abi = "c")]
/// impl MyTrait for Calculator {
///     fn add(&self, a: i32, b: i32) -> i32 {
///         a + b
///     }
/// }
/// ```
///
/// This will generate extern "C" functions that can be called from other languages.
#[proc_macro_attribute]
pub fn impl_extern_trait(args: TokenStream, input: TokenStream) -> TokenStream {
    let (crate_name_str, abi) = match parse_extern_trait_args(args) {
        Ok((name, abi)) => (name, abi),
        Err(error_msg) => {
            bail!(Span::call_site(), error_msg);
        }
    };
    let input = parse_macro_input!(input as ItemImpl);
    let mut extern_fn_list = vec![];

    let struct_name = input.self_ty.clone();
    let trait_name = input.clone().trait_.unwrap().1;

    for item in &input.items {
        if let syn::ImplItem::Fn(func) = item {
            let fn_name_raw = &func.sig.ident;
            let fn_name = extern_fn_name(&crate_name_str, fn_name_raw);

            let inputs = &func.sig.inputs;
            let output = &func.sig.output;
            let generics = &func.sig.generics;
            let unsafety = &func.sig.unsafety;
            // preserve attributes from the original impl method (e.g. #[cfg], docs)
            let attrs = &func.attrs;

            let extern_abi = if abi == "rust" { "Rust" } else { "C" };

            let mut param_names = vec![];
            let mut param_types = vec![];

            for input in inputs {
                if let syn::FnArg::Typed(pat_type) = input {
                    param_names.push(&pat_type.pat);
                    param_types.push(&pat_type.ty);
                }
            }
            let mut body = quote! {
                <#struct_name as #trait_name>::#fn_name_raw(#(#param_names),*)
            };

            if unsafety.is_some() {
                body = quote! { unsafe { #body } };
            }
            extern_fn_list.push(quote! {
                #(#attrs)*
                /// `trait-ffi` generated extern function.
                #[unsafe(no_mangle)]
                pub #unsafety extern #extern_abi fn #fn_name #generics (#inputs) #output {
                    #body
                }
            });
        }
    }

    quote! {
        #input
        #(#extern_fn_list)*
    }
    .into()
}
