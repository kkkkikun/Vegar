#![doc(test(attr(deny(warnings))))]
#![doc(test(attr(allow(dead_code))))]

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::ToTokens;
use syn::parse_macro_input;

macro_rules! bail {
    ($span:expr, $($tt:tt)*) => {
        return Err(syn::Error::new_spanned($span, format!($($tt)*)))
    };
}

mod volatile;

/// A derive macro for method-based accesses to volatile structures.
///
/// This macro allows you to access the fields of a volatile structure via methods that enforce access limitations.
/// It is also more easily chainable than `map_field`.
///
/// <div class="warning">
///
/// This macro generates and implements a new `{T}VolatileFieldAccess` trait, that you have to import if used from other modules.
/// Currently, the trait is only implemented for `VolatilePtr<'_, _, ReadWrite>`.
///
/// </div>
///
/// # Examples
///
/// ```
/// use volatile::access::ReadOnly;
/// use volatile::{VolatileFieldAccess, VolatileRef};
///
/// #[repr(C)]
/// #[derive(VolatileFieldAccess, Default)]
/// pub struct DeviceConfig {
///     feature_select: u32,
///     #[access(ReadOnly)]
///     feature: u32,
/// }
///
/// let mut device_config = DeviceConfig::default();
/// let mut volatile_ref = VolatileRef::from_mut_ref(&mut device_config);
/// let volatile_ptr = volatile_ref.as_mut_ptr();
///
/// volatile_ptr.feature_select().write(42);
/// assert_eq!(volatile_ptr.feature_select().read(), 42);
///
/// // This does not compile, because we specified `#[access(ReadOnly)]` for this field.
/// // volatile_ptr.feature().write(42);
///
/// // A real device might have changed the value, though.
/// assert_eq!(volatile_ptr.feature().read(), 0);
///
/// // You can also use shared references.
/// let volatile_ptr = volatile_ref.as_ptr();
/// assert_eq!(volatile_ptr.feature_select().read(), 42);
/// // This does not compile, because `volatile_ptr` is `ReadOnly`.
/// // volatile_ptr.feature_select().write(42);
/// ```
///
/// # Details
///
/// This macro generates a new trait (`{T}VolatileFieldAccess`) and implements it for `VolatilePtr<'a, T, ReadWrite>`.
/// The example above results in (roughly) the following code:
///
/// ```
/// # #[repr(C)]
/// # pub struct DeviceConfig {
/// #     feature_select: u32,
/// #     feature: u32,
/// # }
/// use volatile::access::{ReadOnly, ReadWrite, RestrictAccess};
/// use volatile::{map_field, VolatilePtr};
///
/// pub trait DeviceConfigVolatileFieldAccess<'a, A> {
///     fn feature_select(self) -> VolatilePtr<'a, u32, A::Restricted>
///     where
///         A: RestrictAccess<ReadWrite>;
///
///     fn feature(self) -> VolatilePtr<'a, u32, A::Restricted>
///     where
///         A: RestrictAccess<ReadOnly>;
/// }
///
/// impl<'a, A> DeviceConfigVolatileFieldAccess<'a, A> for VolatilePtr<'a, DeviceConfig, A> {
///     fn feature_select(self) -> VolatilePtr<'a, u32, A::Restricted>
///     where
///         A: RestrictAccess<ReadWrite>
///     {
///         map_field!(self.feature_select).restrict()
///     }
///
///     fn feature(self) -> VolatilePtr<'a, u32, A::Restricted>
///     where
///         A: RestrictAccess<ReadOnly>
///     {
///         map_field!(self.feature).restrict()
///     }
/// }
/// ```
#[proc_macro_derive(VolatileFieldAccess, attributes(access))]
pub fn derive_volatile(item: TokenStream) -> TokenStream {
    match volatile::derive_volatile(parse_macro_input!(item)) {
        Ok(items) => {
            let mut tokens = TokenStream2::new();
            for item in &items {
                item.to_tokens(&mut tokens);
            }
            tokens.into()
        }
        Err(e) => e.to_compile_error().into(),
    }
}
