use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, spanned::Spanned, FnArg,
    Item,
};
extern crate proc_macro;

#[proc_macro_attribute]
pub fn labt_lua(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let function = match parse_macro_input!(item as Item) {
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

    let params = &function.sig.inputs;

    let output: TokenStream = quote! {
         #function_visibility fn #name(lua: &mlua::Lua, table: &mut mlua::Table) -> mlua::Result<()> {
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
