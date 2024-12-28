use anyhow::Context;
use dialoguer::{self, theme::ColorfulTheme, Confirm};
use labt_proc_macro::labt_lua;
use mlua::Lua;

use super::MluaAnyhowWrapper;

/// prompt a user for a yes or no answer.
/// Returns the selected choice
#[labt_lua]
fn confirm(_lua: &Lua, (prompt, default): (String, Option<bool>)) {
    let mut p = Confirm::new().with_prompt(prompt);
    if let Some(default) = default {
        p = p.default(default).show_default(true);
    }

    let response = p
        .interact()
        .context("Failed to show confirm prompt.")
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(response)
}
/// prompt a user for a yes or no answer.
/// The user can cancel responding to the answer by pressing esc
/// Returns the selected choice or None if canceled
#[labt_lua]
fn confirm_optional(_lua: &Lua, (prompt, default): (String, Option<bool>)) {
    let theme = ColorfulTheme::default();
    let mut p = Confirm::with_theme(&theme).with_prompt(prompt);
    if let Some(default) = default {
        p = p.default(default).show_default(true);
    }

    let response = p
        .interact_opt()
        .context("Failed to show optional confirm prompt.")
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(response)
}

/// Generates prompt table and loads all its api functions
///
/// # Errors
///
/// This function will return an error if adding functions to prompt table fails
/// or the underlying lua operations return errors.
pub fn load_prompt_table(lua: &mut Lua) -> anyhow::Result<()> {
    let table = lua.create_table()?;
    confirm(lua, &table)?;
    confirm_optional(lua, &table)?;
    lua.globals().set("prompt", table)?;
    Ok(())
}
