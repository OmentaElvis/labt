use std::any::Any;

use anyhow::Context;
use labt_proc_macro::labt_lua;
use mlua::{Chunk, Function, Lua, MultiValue, Table, Value, Variadic};
use serde::Serialize;

use super::MluaAnyhowWrapper;

const FUNCTIONS: &str = "functions";
const REF: &str = "ref";
const ARGS: &str = "args";

#[labt_lua]
fn add_task(lua: &Lua, (table_self, callable, args): (Table, Function, MultiValue)) {
    let functions: Table = table_self.get(FUNCTIONS)?;
    let function: Table = lua.create_table()?;
    function.set(REF, callable)?;

    let args_table: Table = lua.create_table()?;
    for arg in args {
        args_table.push(arg)?;
    }

    function.set(ARGS, args_table)?;

    functions.push(function)?;

    Ok(table_self)
}

async fn run_tasks<'lua>(lua: &Lua, functions: Table<'lua>) -> mlua::Result<()> {
    // let mut handles = Vec::new();

    for table in functions.sequence_values() {
        let table: Table = table?;
        let function: Function = table.get(REF)?;
        let code = function.dump(true);

        let args: Table = table.get(ARGS)?;
        let ser = mlua::serde::Serializer::new(lua);

        let mut varargs = Variadic::new();
        for arg in args.sequence_values() {
            let arg: Value = arg.unwrap();
            varargs.push(arg);
        }

        // handles.push(tokio::spawn(async move {
        let state = Lua::new();

        let chunk = state.load(code);
        let ret: MultiValue = chunk.call(varargs).unwrap();
        // }));
    }
    // for handle in handles {
    //     handle.await;
    // }

    Ok(())
}

#[labt_lua]
fn execute(lua: &Lua, table_self: Table) {
    let functions: Table = table_self.get(FUNCTIONS)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to build tokio runtime")
        .map_err(MluaAnyhowWrapper::external)?;

    let result: mlua::Result<()> = runtime.block_on(run_tasks(lua, functions));
    Ok(())
}

#[labt_lua]
fn new(lua: &Lua) {
    let pool = lua.create_table()?;
    let functions = lua.create_table()?;

    pool.set(FUNCTIONS, functions)?;

    // add functions
    add_task(lua, &pool)?;
    execute(lua, &pool)?;

    Ok(pool)
}

pub fn load_thread_pool_table(lua: &mut Lua) -> anyhow::Result<()> {
    let table = lua.create_table()?;

    new(lua, &table)?;

    lua.globals().set("thread_pool", table)?;
    Ok(())
}
