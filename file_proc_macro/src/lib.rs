extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

fn gen_mapping(field: &syn::Field) -> Vec<quote::__private::TokenStream> {
    let syn::Field { attrs, .. } = field;

    let ident = field.ident.as_ref().unwrap();
    attrs
        .iter()
        .map(|attr| match &attr.meta {
            syn::Meta::NameValue(syn::MetaNameValue {
                path,
                eq_token: _,
                value: syn::Expr::Lit(v),
            }) if path.is_ident("fsfile") => {
                if let syn::Lit::Str(v) = &v.lit {
                    v.value()
                } else {
                    panic!("gen mapping found unexpected '{:?}'", v);
                }
            }
            _ => panic!("unexpected meta '{:?}", attr.meta),
        })
        .map(|key| {
            quote! {
                #key => &*self.#ident
            }
        })
        .collect::<Vec<_>>()
}

fn gen_mappings(fields: syn::Fields) -> Vec<quote::__private::TokenStream> {
    fields.iter().flat_map(gen_mapping).collect()
}

#[proc_macro_derive(FsFile, attributes(fsfile, fail))]
pub fn file_derive(input: TokenStream) -> TokenStream {
    let input: DeriveInput = parse_macro_input!(input);
    let mappings = match input.data {
        syn::Data::Struct(syn::DataStruct { fields, .. }) => gen_mappings(fields),
        _ => panic!("Unexpected input: {:?}", input.data),
    };
    let ident = &input.ident;
    let generics = &input.generics;

    let output = quote! {
        impl #generics FsFile for #ident #generics {}
        impl #generics Index<&str> for #ident #generics {
            type Output = str;

            fn index(&self, index: &str) -> &Self::Output {
                match index {
                    #(#mappings,)*
                    _ => unimplemented!("No mapping for {} in {}", index, stringify!(#ident)),
                }
            }

        }

    };
    output.into()
}
