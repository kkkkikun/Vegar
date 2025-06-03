use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, PathArguments, parse_macro_input};

#[proc_macro_attribute]
pub fn syscall_trace(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut function = parse_macro_input!(item as ItemFn);
    let signature = &function.sig;
    let fn_name = &signature.ident;
    let fn_args: Vec<_> = signature
        .inputs
        .iter()
        .filter_map(|arg| {
            // 仅解析命名的函数参数（如 `x: i32`，跳过 `self` 或模式匹配参数）
            match arg {
                syn::FnArg::Typed(pat_type) => {
                    // 提取参数名（例如 `x`）
                    let arg_name = match &*pat_type.pat {
                        syn::Pat::Ident(pat_ident) => &pat_ident.ident,
                        _ => return None, // 跳过复杂模式（如元组解构）
                    };
                    if arg_name == "tf" {
                        return None; // 跳过 `tf` 参数 (trap frame)
                    }
                    // 提取参数类型（例如 `i32`）
                    let arg_type = &pat_type.ty;
                    Some((arg_name, arg_type))
                }
                _ => None, // 忽略 `self` 等特殊参数
            }
        })
        .collect();

    let arg_patterns_in: Vec<_> = fn_args
        .iter()
        .map(|(arg_name, arg_type)| match &***arg_type {
            syn::Type::Path(type_path) => {
                let outer_type = type_path.path.segments.last().unwrap();
                let type_name = &outer_type.ident;
                if type_name == "UserInPtr" || type_name == "UserInOutPtr" {
                    if let PathArguments::AngleBracketed(args) = &outer_type.arguments {
                        let inner_generic_argument = args.args.first().unwrap();
                        if let syn::GenericArgument::Type(inner_ty) = inner_generic_argument {
                            if let syn::Type::Path(inner_path) = inner_ty {
                                let inner_segment = inner_path.path.segments.last().unwrap();
                                if inner_segment.ident == "c_char" {
                                    return quote! { #arg_name.fmt_trace_as_str() };
                                }
                            }
                        }
                    }
                    quote! { #arg_name.fmt_trace_content() }
                } else if type_name == "UserOutPtr" {
                    quote! { #arg_name.fmt_trace() }
                } else {
                    quote! { #arg_name }
                }
            }
            _ => quote! { #arg_name },
        })
        .collect();

    let arg_patterns_out: Vec<_> = fn_args
        .iter()
        .map(|(arg_name, arg_type)| match &***arg_type {
            syn::Type::Path(type_path) => {
                let outer_type = type_path.path.segments.last().unwrap();
                let type_name = &outer_type.ident;
                if type_name == "UserInPtr"
                    || type_name == "UserOutPtr"
                    || type_name == "UserInOutPtr"
                {
                    if let PathArguments::AngleBracketed(args) = &outer_type.arguments {
                        let inner_generic_argument = args.args.first().unwrap();
                        if let syn::GenericArgument::Type(inner_ty) = inner_generic_argument {
                            if let syn::Type::Path(inner_path) = inner_ty {
                                let inner_segment = inner_path.path.segments.last().unwrap();
                                if inner_segment.ident == "c_char" {
                                    return quote! { #arg_name.fmt_trace_as_str() };
                                }
                            }
                        }
                    }
                    quote! { #arg_name.fmt_trace_content() }
                } else {
                    quote! { #arg_name }
                }
            }
            _ => quote! { #arg_name },
        })
        .collect();

    let arg_names: Vec<_> = fn_args.iter().map(|(name, _)| quote! { #name }).collect();
    let arg_list_pattern = arg_names
        .iter()
        .map(|name| format!("{} = {{}}", name.to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    let format_pattern_in = format!("[syscall] <= {}({})", fn_name, arg_list_pattern);
    let format_pattern_out = format!("[syscall] => {}({}) = {{}}", fn_name, arg_list_pattern);

    let fn_body = &function.block;
    function.block = syn::parse2(quote! {{
        debug!(#format_pattern_in #(, #arg_patterns_in)*);

        let __result = (|| {
            #fn_body
        })();

        use alloc::format;

        let __linux_result = match __result {
            Ok(ref value) => {
                format!("{:?}", value)
            }
            Err(ref error) => {
                format!("{:?}", error)
            }
        };
        debug!(#format_pattern_out #(, #arg_patterns_out)*, __linux_result);
        __result
    }})
    .unwrap();
    quote! {
        #function
    }
    .into()
}
