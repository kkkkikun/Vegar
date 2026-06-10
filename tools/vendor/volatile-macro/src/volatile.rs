use quote::format_ident;
use syn::punctuated::Punctuated;
use syn::{
    parse_quote, Attribute, Fields, Ident, Item, ItemImpl, ItemStruct, ItemTrait, Meta, Path,
    Result, Signature, Token, Visibility,
};

fn validate_input(input: &ItemStruct) -> Result<()> {
    if !matches!(&input.fields, Fields::Named(_)) {
        bail!(
            &input.fields,
            "#[derive(VolatileFieldAccess)] can only be used on structs with named fields"
        );
    }

    if !input.generics.params.is_empty() {
        bail!(
            &input.generics,
            "#[derive(VolatileFieldAccess)] cannot be used with generic structs"
        );
    }

    let mut valid_repr = false;
    for attr in &input.attrs {
        if attr.path().is_ident("repr") {
            let nested = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            for meta in nested {
                if let Meta::Path(path) = meta {
                    if path.is_ident("C") || path.is_ident("transparent") {
                        valid_repr = true;
                    }
                }
            }
        }
    }
    if !valid_repr {
        bail!(
            &input.ident,
            "#[derive(VolatileFieldAccess)] structs must be `#[repr(C)]` or `#[repr(transparent)]`"
        );
    }

    Ok(())
}

struct ParsedInput {
    attrs: Vec<Attribute>,
    vis: Visibility,
    trait_ident: Ident,
    struct_ident: Ident,
    method_attrs: Vec<Vec<Attribute>>,
    sigs: Vec<Signature>,
}

fn parse_input(input: &ItemStruct) -> Result<ParsedInput> {
    let mut attrs = vec![];
    for attr in &input.attrs {
        if attr.path().is_ident("doc") {
            attrs.push(attr.clone());
        }
    }

    let mut method_attrs = vec![];
    for field in &input.fields {
        let mut attrs = vec![];
        for attr in &field.attrs {
            if attr.path().is_ident("doc") {
                attrs.push(attr.clone());
            }
        }
        method_attrs.push(attrs);
    }

    let mut sigs = vec![];
    for field in &input.fields {
        let ident = field.ident.as_ref().unwrap();
        let ty = &field.ty;

        let mut access: Path = parse_quote! { ::volatile::access::ReadWrite };
        for attr in &field.attrs {
            if attr.path().is_ident("access") {
                access = attr.parse_args()?;
            }
        }

        let sig = parse_quote! {
            fn #ident(self) -> ::volatile::VolatilePtr<'a, #ty, A::Restricted>
            where
                A: ::volatile::access::RestrictAccess<#access>
        };
        sigs.push(sig);
    }

    Ok(ParsedInput {
        attrs,
        vis: input.vis.clone(),
        trait_ident: format_ident!("{}VolatileFieldAccess", input.ident),
        struct_ident: input.ident.clone(),
        method_attrs,
        sigs,
    })
}

fn emit_trait(
    ParsedInput {
        attrs,
        vis,
        trait_ident,
        method_attrs,
        sigs,
        ..
    }: &ParsedInput,
) -> ItemTrait {
    parse_quote! {
        #(#attrs)*
        #[allow(non_camel_case_types)]
        #vis trait #trait_ident <'a, A> {
            #(
                #(#method_attrs)*
                #sigs;
            )*
        }
    }
}

fn emit_impl(
    ParsedInput {
        trait_ident,
        struct_ident,
        sigs,
        ..
    }: &ParsedInput,
) -> ItemImpl {
    let fields = sigs.iter().map(|sig| &sig.ident);

    parse_quote! {
        #[automatically_derived]
        impl<'a, A> #trait_ident<'a, A> for ::volatile::VolatilePtr<'a, #struct_ident, A> {
            #(
                #sigs,
                {
                    ::volatile::map_field!(self.#fields).restrict()
                }
            )*
        }
    }
}

pub fn derive_volatile(input: ItemStruct) -> Result<Vec<Item>> {
    validate_input(&input)?;
    let parsed_input = parse_input(&input)?;
    let item_trait = emit_trait(&parsed_input);
    let item_impl = emit_impl(&parsed_input);
    Ok(vec![Item::Trait(item_trait), Item::Impl(item_impl)])
}

#[cfg(test)]
mod tests {
    use quote::{quote, ToTokens};

    use super::*;

    #[test]
    fn test_derive() -> Result<()> {
        let input = parse_quote! {
            /// Struct documentation.
            ///
            /// This is a wonderful struct.
            #[repr(C)]
            #[derive(VolatileFieldAccess, Default)]
            pub struct DeviceConfig {
                feature_select: u32,

                /// Feature.
                ///
                /// This is a good field.
                #[access(ReadOnly)]
                feature: u32,
            }
        };

        let result = derive_volatile(input)?;

        let expected_trait = quote! {
            /// Struct documentation.
            ///
            /// This is a wonderful struct.
            #[allow(non_camel_case_types)]
            pub trait DeviceConfigVolatileFieldAccess<'a, A> {
                fn feature_select(self) -> ::volatile::VolatilePtr<'a, u32, A::Restricted>
                where
                    A: ::volatile::access::RestrictAccess<::volatile::access::ReadWrite>;

                /// Feature.
                ///
                /// This is a good field.
                fn feature(self) -> ::volatile::VolatilePtr<'a, u32, A::Restricted>
                where
                    A: ::volatile::access::RestrictAccess<ReadOnly>;
            }
        };

        let expected_impl = quote! {
            #[automatically_derived]
            impl<'a, A> DeviceConfigVolatileFieldAccess<'a, A> for ::volatile::VolatilePtr<'a, DeviceConfig, A> {
                fn feature_select(self) -> ::volatile::VolatilePtr<'a, u32, A::Restricted>
                where
                    A: ::volatile::access::RestrictAccess<::volatile::access::ReadWrite>,
                {
                    ::volatile::map_field!(self.feature_select).restrict()
                }

                fn feature(self) -> ::volatile::VolatilePtr<'a, u32, A::Restricted>
                where
                    A: ::volatile::access::RestrictAccess<ReadOnly>,
                {
                    ::volatile::map_field!(self.feature).restrict()
                }
            }
        };

        assert_eq!(
            expected_trait.to_string(),
            result[0].to_token_stream().to_string()
        );
        assert_eq!(
            expected_impl.to_string(),
            result[1].to_token_stream().to_string()
        );

        Ok(())
    }

    #[test]
    fn test_align() -> Result<()> {
        let input = parse_quote! {
            #[repr(C, align(8))]
            #[derive(VolatileFieldAccess)]
            pub struct DeviceConfig {}
        };

        let result = derive_volatile(input)?;

        let expected_trait = quote! {
            #[allow(non_camel_case_types)]
            pub trait DeviceConfigVolatileFieldAccess<'a, A> {}
        };

        let expected_impl = quote! {
            #[automatically_derived]
            impl<'a, A> DeviceConfigVolatileFieldAccess<'a, A> for ::volatile::VolatilePtr<'a, DeviceConfig, A> {}
        };

        assert_eq!(
            expected_trait.to_string(),
            result[0].to_token_stream().to_string()
        );
        assert_eq!(
            expected_impl.to_string(),
            result[1].to_token_stream().to_string()
        );

        Ok(())
    }
}
