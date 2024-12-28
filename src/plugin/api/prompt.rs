use std::{fmt::Display, str::FromStr};

use anyhow::Context;
use dialoguer::{self, theme::ColorfulTheme, Confirm, Input, MultiSelect, Password, Select};
use labt_proc_macro::labt_lua;
use mlua::{Function, IntoLua, Lua, Number};

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

fn input_prompt<T>(
    prompt: String,
    default: Option<T>,
    validator: Option<Function>,
) -> mlua::Result<T>
where
    T: for<'lua> IntoLua<'lua> + ToOwned<Owned = T> + FromStr + Clone + Display,
    <T as FromStr>::Err: ToString,
{
    let theme = ColorfulTheme::default();
    let mut p = Input::<T>::with_theme(&theme).with_prompt(prompt);
    if let Some(default) = default {
        p = p.default(default).show_default(true);
    }

    if let Some(validator) = validator {
        p = p.validate_with(move |input: &T| {
            let res = validator
                .call::<T, Option<String>>(input.to_owned())
                .context("Failed to execute lua validator callback function.")
                .map_err(MluaAnyhowWrapper::external)
                .unwrap();

            if let Some(err) = res {
                Err(err)
            } else {
                Ok(())
            }
        })
    }

    let response = p
        .interact_text()
        .context("Failed to show input prompt.")
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(response)
}

/// Prompt the user for a string input.
/// You can set a default value
/// You can provide an optional validator callback that is going to verify the input and return an error string if invalid or nil if valid.
/// Returns the entered string
#[labt_lua]
fn input(_lua: &Lua, (prompt, default, validator): (String, Option<String>, Option<Function>)) {
    input_prompt::<String>(prompt, default, validator)
}
/// Prompt the user for a number input.
/// You can set a default value
/// You can provide an optional validator callback that is going to verify the input and return an error string if invalid or nil if valid.
/// Returns the entered number
#[labt_lua]
fn input_number(
    _lua: &Lua,
    (prompt, default, validator): (String, Option<Number>, Option<Function>),
) {
    input_prompt::<Number>(prompt, default, validator)
}

/// Prompt the user for a hidden input.
/// You can provide an optional validator callback that is going to verify the input and return an error string if invalid or nil if valid.
/// Returns the entered string
#[labt_lua]
fn input_password(_lua: &Lua, (prompt, validator): (String, Option<Function>)) {
    let theme = ColorfulTheme::default();
    let mut p = Password::with_theme(&theme).with_prompt(prompt);
    if let Some(validator) = validator {
        p = p.validate_with(move |input: &String| {
            let res = validator
                .call::<String, Option<String>>(input.to_owned())
                .context("Failed to execute lua validator callback function.")
                .map_err(MluaAnyhowWrapper::external)
                .unwrap();

            if let Some(err) = res {
                Err(err)
            } else {
                Ok(())
            }
        })
    }

    let response = p
        .interact()
        .context("Failed to show password input prompt.")
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(response)
}
#[labt_lua]
fn select(_lua: &Lua, (prompt, options, default): (String, Vec<String>, Option<usize>)) {
    let theme = ColorfulTheme::default();
    let mut p = Select::with_theme(&theme).with_prompt(prompt);
    for option in options {
        p = p.item(option);
    }

    if let Some(default) = default {
        let d = default.saturating_sub(1);
        p = p.default(d);
    }

    let response = p
        .interact()
        .context("Failed to show selection prompt.")
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(response + 1)
}
#[labt_lua]
fn multi_select(_lua: &Lua, (prompt, options, default): (String, Vec<String>, Option<Vec<bool>>)) {
    let theme = ColorfulTheme::default();

    let mut p = MultiSelect::with_theme(&theme).with_prompt(prompt);
    for option in options {
        p = p.item(option);
    }

    if let Some(default) = default {
        p = p.defaults(&default);
    }

    let response = p
        .interact()
        .context("Failed to show selection prompt.")
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(response.iter().map(|i| i + 1).collect::<Vec<usize>>())
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
    input(lua, &table)?;
    input_number(lua, &table)?;
    input_password(lua, &table)?;
    select(lua, &table)?;
    multi_select(lua, &table)?;
    lua.globals().set("prompt", table)?;
    Ok(())
}
