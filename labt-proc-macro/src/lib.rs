use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, punctuated::Punctuated, spanned::Spanned, token::Paren, FnArg, Item,
    PatTuple, PatType, Token, TypeTuple,
};
extern crate proc_macro;

#[proc_macro_attribute]
pub fn labt_lua(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut function = match parse_macro_input!(item as Item) {
        Item::Fn(item) => item,
        _ => panic!("This attribute is only applicable to functions"),
    };

    let name = &function.sig.ident;

    let sig = &function.sig;

    let function_visibility = &function.vis;
    let block = &function.block;
    let function_return = &function.sig.output;

    // obtain the first argument
    if let Some(first) = sig.inputs.first() {
        match first {
            FnArg::Typed(arg) => arg,
            _ => {
                return syn::Error::new(
                    first.span(),
                    "Only functions are allowed, methods are not supported",
                )
                .to_compile_error()
                .into()
            }
        }
    } else {
        return syn::Error::new(
            name.span(),
            "Incorrect function signature, at least the Lua context is required! as the first argument",
        )
        .to_compile_error()
        .into();
    };

    if function.sig.inputs.len() < 2 {
        // less than two args specified, add an empty tuple since the user
        // doesnt require args from lua
        function.sig.inputs.push(FnArg::Typed(PatType {
            pat: Box::new(syn::Pat::Tuple(PatTuple {
                attrs: vec![],
                paren_token: Paren::default(),
                elems: Punctuated::new(),
            })),
            ty: Box::new(syn::Type::Tuple(TypeTuple {
                paren_token: Paren::default(),
                elems: Punctuated::new(),
            })),
            attrs: vec![],
            colon_token: Token![:](function.sig.inputs.span()),
        }));
    }
    let params = &function.sig.inputs;

    let output: TokenStream = quote! {
         #function_visibility fn #name(lua: &mlua::Lua, table: &mlua::Table) -> mlua::Result<()> {
            let function = lua.create_function(move |#params|  #function_return
                #block
            )?;

            table.set(stringify!(#name), function)?;
            Ok(())
        }
    }
    .into();
    println!("{}", output);

    output
}
