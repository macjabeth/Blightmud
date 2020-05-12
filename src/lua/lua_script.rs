use super::constants::*;
use super::user_data::*;
use super::util::*;
use crate::event::Event;
use rlua::{Lua, Result as LuaResult};
use std::io::prelude::*;
use std::{error::Error, fs::File, result::Result, sync::mpsc::Sender};
use strip_ansi_escapes::strip as strip_ansi;

pub struct LuaScript {
    state: Lua,
    writer: Sender<Event>,
}

fn create_default_lua_state(writer: Sender<Event>) -> Lua {
    let state = Lua::new();

    let blight = BlightMud::new(writer);
    state
        .context(|ctx| -> LuaResult<()> {
            let globals = ctx.globals();
            globals.set("blight", blight)?;

            let alias_table = ctx.create_table()?;
            globals.set(ALIAS_TABLE, alias_table)?;
            let trigger_table = ctx.create_table()?;
            globals.set(TRIGGER_TABLE, trigger_table)?;
            let prompt_trigger = ctx.create_table()?;
            globals.set(PROMPT_TRIGGER_TABLE, prompt_trigger)?;
            let gmcp_listener_table = ctx.create_table()?;
            globals.set(GMCP_LISTENER_TABLE, gmcp_listener_table)?;

            Ok(())
        })
        .unwrap();
    state
}

impl LuaScript {
    pub fn new(main_thread_writer: Sender<Event>) -> Self {
        Self {
            state: create_default_lua_state(main_thread_writer.clone()),
            writer: main_thread_writer,
        }
    }

    pub fn reset(&mut self) {
        self.state = create_default_lua_state(self.writer.clone());
    }

    pub fn check_for_alias_match(&self, input: &str) -> bool {
        let mut response = false;
        self.state.context(|ctx| {
            let alias_table: rlua::Table = ctx.globals().get(ALIAS_TABLE).unwrap();
            for pair in alias_table.pairs::<rlua::Value, rlua::AnyUserData>() {
                let (_, alias) = pair.unwrap();
                let rust_alias = &alias.borrow::<Alias>().unwrap();
                let regex = &rust_alias.regex;
                if rust_alias.enabled && regex.is_match(input) {
                    let cb: rlua::Function = alias.get_user_value().unwrap();
                    let captures: Vec<String> = regex
                        .captures(input)
                        .unwrap()
                        .iter()
                        .map(|c| match c {
                            Some(m) => m.as_str().to_string(),
                            None => String::new(),
                        })
                        .collect();
                    if let Err(msg) = cb.call::<_, ()>(captures) {
                        output_stack_trace(&self.writer, &msg.to_string());
                    }
                    response = true;
                }
            }
        });
        response
    }

    pub fn check_for_trigger_match(&self, input: &str) -> bool {
        self.check_trigger_match(input, TRIGGER_TABLE)
    }

    pub fn check_for_prompt_trigger_match(&self, input: &str) -> bool {
        self.check_trigger_match(input, PROMPT_TRIGGER_TABLE)
    }

    fn check_trigger_match(&self, input: &str, table: &str) -> bool {
        let clean_bytes = strip_ansi(input.as_bytes()).unwrap();
        let input = &String::from_utf8_lossy(&clean_bytes);
        let mut response = false;
        self.state.context(|ctx| {
            let trigger_table: rlua::Table = ctx.globals().get(table).unwrap();
            for pair in trigger_table.pairs::<rlua::Value, rlua::AnyUserData>() {
                let (_, trigger) = pair.unwrap();
                let rust_trigger = &trigger.borrow::<Trigger>().unwrap();
                if rust_trigger.enabled && rust_trigger.regex.is_match(input) {
                    let cb: rlua::Function = trigger.get_user_value().unwrap();
                    let captures: Vec<String> = rust_trigger
                        .regex
                        .captures(input)
                        .unwrap()
                        .iter()
                        .map(|c| match c {
                            Some(m) => m.as_str().to_string(),
                            None => String::new(),
                        })
                        .collect();
                    if let Err(msg) = cb.call::<_, ()>(captures) {
                        output_stack_trace(&self.writer, &msg.to_string());
                    }
                    response = rust_trigger.gag;
                }
            }
        });
        response
    }

    pub fn receive_gmcp(&mut self, data: &str) {
        let split = data
            .splitn(2, ' ')
            .map(String::from)
            .collect::<Vec<String>>();
        let msg_type = &split[0];
        let content = &split[1];
        self.state
            .context(|ctx| {
                let listener_table: rlua::Table = ctx.globals().get(GMCP_LISTENER_TABLE).unwrap();
                if let Ok(func) = listener_table.get::<_, rlua::Function>(msg_type.clone()) {
                    func.call::<_, ()>(content.clone())?;
                }
                rlua::Result::Ok(())
            })
            .ok();
    }

    pub fn load_script(&mut self, path: &str) -> Result<(), Box<dyn Error>> {
        let mut file = File::open(path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        if let Err(msg) = self
            .state
            .context(|ctx| -> LuaResult<()> { ctx.load(&content).set_name(path)?.exec() })
        {
            output_stack_trace(&self.writer, &msg.to_string());
        }
        Ok(())
    }

    pub fn on_connect(&mut self) {
        self.state
            .context(|ctx| -> Result<(), rlua::Error> {
                if let Ok(callback) = ctx
                    .globals()
                    .get::<_, rlua::Function>(ON_CONNCTION_CALLBACK)
                {
                    callback.call::<_, ()>(())
                } else {
                    Ok(())
                }
            })
            .unwrap();
    }

    pub fn on_gmcp_ready(&mut self) {
        self.state
            .context(|ctx| -> Result<(), rlua::Error> {
                if let Ok(callback) = ctx
                    .globals()
                    .get::<_, rlua::Function>(ON_GMCP_READY_CALLBACK)
                {
                    callback.call::<_, ()>(())
                } else {
                    Ok(())
                }
            })
            .unwrap();
    }
}