//! `br mcp`: BR Foto for AI agents over the Model Context Protocol.
//!
//! LightCraft already ships a complete MCP server (`lightcraft-mcp`): import, query, develop,
//! render and export tools, plus one `cmd_*` tool per engine command. BR keeps all of it and adds:
//!
//! - [`BrBackend`]: LightCraft's in-process [`Headless`] backend with the `br.*` commands of
//!   [`br_photo::commands`] routed in, so they also work through `command_run` and, with `--full`,
//!   as `cmd_br_*` tools.
//! - [`BrServer`]: a front for LightCraft's [`Server`] that names itself `br`, puts BR's
//!   instructions first and lists typed `br_*` tools (proper schemas, a preview image) ahead of
//!   LightCraft's own. Everything else is answered by LightCraft's server unchanged.
//!
//! The protocol is newline-delimited JSON-RPC 2.0 over stdio.

use std::io::{BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};

use br_photo::BrState;
use br_photo::adjust::Slider;
use br_photo::commands;
use br_photo::crop::Format;
use br_photo::export::Preset;
use lightcraft_mcp::{Backend, Headless, ProgressHook, Server, ToolResult, call_tool};
use serde_json::{Map, Value, json};

/// Prefix of BR's own tools.
pub const TOOL_PREFIX: &str = "br_";

/// What `br mcp` tells the agent first.
pub const INSTRUCTIONS: &str = "BR is a media suite; this server is BR Foto, simple photo editing on LightCraft's engine. \
Start with the br_* tools: they work on the active photo unless you pass `id`. A typical edit: `import` the files (or \
`query_photos` / `select_photos` to pick one), `br_looks` then `br_look` for a style, `br_adjust` for light, contrast, \
color, warmth, sharpness and vignette (-100..100, 0 = unchanged), `br_crop` for a format (1:1, 4:5, 9:16, 16:9), \
`br_preview` to look at the result, and `br_export` with a preset (instagram 1080x1350, story 1080x1920, square, wide, \
full). Every edit is one undo step (`br_undo`). For finer control use LightCraft's tools below (`list_controls`, \
`set_develop`, `command_list`, `command_run`).";

/// LightCraft's headless backend plus BR's `br.*` commands.
pub struct BrBackend {
    pub inner: Headless,
    pub state: BrState,
    /// How the session was opened, for the agent ("library at …", "demo photos", …).
    pub origin: String,
}

impl BrBackend {
    pub fn new(inner: Headless, origin: impl Into<String>) -> Self {
        Self { inner, state: BrState::default(), origin: origin.into() }
    }
}

impl Backend for BrBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if matches!(method, "engine.execute" | "ui.menu.invoke" | "command")
            && let Some(id) = params.get("command").or(params.get("id")).and_then(Value::as_str).filter(|c| commands::is_br(c))
        {
            let id = id.to_string();
            let p = params.get("params").cloned().unwrap_or(Value::Null);
            return commands::execute(&mut self.inner.session, &mut self.state, &id, &p).map_err(|e| e.to_string());
        }
        let mut r = self.inner.call(method, params)?;
        if method == "engine.commands"
            && let Value::Array(a) = &mut r
        {
            a.extend(commands::list(&self.inner.session));
        }
        Ok(r)
    }

    fn has_ui(&self) -> bool {
        self.inner.has_ui()
    }

    fn describe(&self) -> String {
        format!("BR headless, {}", self.origin)
    }

    fn set_progress(&mut self, hook: Option<ProgressHook>) {
        self.inner.set_progress(hook);
    }
}

fn id_prop() -> Value {
    json!({"type": "integer", "description": "Photo id (default: the active photo)"})
}

fn tool(name: &str, title: &str, description: &str, properties: Value, required: &[&str], read_only: bool) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false},
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": false, "idempotentHint": read_only, "openWorldHint": false},
    })
}

/// BR's typed tools.
pub fn tools() -> Vec<Value> {
    let mut sliders = Map::new();
    for k in Slider::ALL {
        sliders.insert(
            k.key().into(),
            json!({"type": "number", "minimum": -100, "maximum": 100, "description": format!("{} ({}): {}", k.label(), k.key(), k.hint())}),
        );
    }
    sliders.insert("id".into(), id_prop());
    let formats: Vec<&str> = Format::ALL.iter().map(|f| f.key()).collect();
    let presets: Vec<&str> = Preset::ALL.iter().map(|p| p.key()).collect();
    let preset_doc: Vec<String> = Preset::ALL.iter().map(|p| format!("{} = {}", p.key(), p.detail())).collect();
    vec![
        tool(
            "br_state",
            "Photo state",
            "The simple view of a photo: file, size, crop format, the look and its amount, the six slider values, and what undo/redo would do.",
            json!({"id": id_prop()}),
            &[],
            true,
        ),
        tool("br_looks", "List looks", "All looks (filter presets): [{id, name, group}].", json!({}), &[], true),
        tool(
            "br_look",
            "Apply look",
            "Apply a look (a filter preset) to a photo. Calling it again for the same look changes its amount instead of stacking it \
             (still one undo step). Amount 0 takes the look off.",
            json!({
                "look": {"type": "string", "description": "Look id or name from br_looks, e.g. lc.golden-hour or \"Golden Hour\""},
                "amount": {"type": "number", "minimum": 0, "maximum": 200, "description": "Strength in percent (default 100, or the look's current amount)"},
                "id": id_prop(),
            }),
            &["look"],
            false,
        ),
        tool(
            "br_adjust",
            "Adjust",
            "Set the simple sliders (absolute values, -100..100, 0 = unchanged). Only the sliders you pass change. One undo step.",
            Value::Object(sliders),
            &[],
            false,
        ),
        tool("br_auto", "Auto", "Automatic exposure, contrast and tone from the photo's own statistics.", json!({"id": id_prop()}), &[], false),
        tool(
            "br_crop",
            "Format",
            "Crop to a format: the largest centred crop in the exact ratio and orientation (9:16 is portrait), in whole pixels. \
             `original` removes the crop.",
            json!({"format": {"type": "string", "enum": formats}, "id": id_prop()}),
            &["format"],
            false,
        ),
        tool(
            "br_export",
            "Export",
            &format!(
                "Export a photo as JPEG with a preset; the photo's own edit is not changed. Presets: {}. Writes to `path` (exact file), \
                 `dir` (named <photo>-<preset>.jpg), or BR's export folder. Returns the written files.",
                preset_doc.join(", ")
            ),
            json!({
                "preset": {"type": "string", "enum": presets},
                "dir": {"type": "string", "description": "Folder to write to"},
                "path": {"type": "string", "description": "Exact output file (.jpg)"},
                "id": id_prop(),
            }),
            &["preset"],
            false,
        ),
        tool(
            "br_preview",
            "Preview",
            "Render the photo as it is edited (crop applied) and return it as an image.",
            json!({
                "id": id_prop(),
                "max_side": {"type": "integer", "minimum": 64, "maximum": 2048, "description": "Longest side in pixels (default 1024)"},
                "format": {"type": "string", "enum": ["jpeg", "png"], "description": "Image format (default jpeg)"},
            }),
            &[],
            true,
        ),
        tool(
            "br_reset",
            "Reset all",
            "Back to how the photo was imported: look, sliders, format and Pro edits.",
            json!({"id": id_prop()}),
            &[],
            false,
        ),
        tool("br_undo", "Undo", "Undo the last edit.", json!({}), &[], false),
        tool("br_redo", "Redo", "Redo the last undone edit.", json!({}), &[], false),
    ]
}

/// The `br.*` command behind a `br_*` tool.
fn command_of(tool: &str) -> Option<&'static str> {
    let name = tool.strip_prefix(TOOL_PREFIX)?;
    commands::COMMANDS.iter().map(|(id, _, _)| *id).find(|id| id.strip_prefix("br.") == Some(name))
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// LightCraft's MCP server with BR in front.
pub struct BrServer {
    server: Server,
    tools: Vec<Value>,
}

impl BrServer {
    /// `full`: also list one `cmd_*` tool per LightCraft command (about 330 tools; they cost an
    /// agent a lot of context, and every command stays reachable through `command_run` anyway).
    pub fn new(backend: BrBackend, full: bool) -> Self {
        Self { server: Server::new(Box::new(backend)).with_command_tools(full), tools: tools() }
    }

    /// The backend, for callers that drive it directly.
    pub fn backend(&mut self) -> &mut dyn Backend {
        self.server.backend()
    }

    /// Serve newline-delimited JSON-RPC until `input` closes.
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            if let Some(reply) = self.handle_line(&line?) {
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Handle one line; the reply line, or `None` for notifications and blank lines.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) if !batch.is_empty() => {
                let replies: Vec<Value> = batch.into_iter().filter_map(|m| self.handle(m)).collect();
                (!replies.is_empty()).then_some(Value::Array(replies))
            }
            Ok(msg @ Value::Object(_)) => self.handle(msg),
            // anything else (an empty batch, a bad message) LightCraft's server answers
            Ok(_) => return self.server.handle_line(line),
            Err(e) => Some(rpc_error(Value::Null, -32700, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    /// Handle one JSON-RPC message.
    pub fn handle(&mut self, msg: Value) -> Option<Value> {
        let method = msg.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
        let id = msg.get("id").cloned();
        match method.as_str() {
            "tools/call" if id.is_some() => {
                let name = msg["params"]["name"].as_str().unwrap_or_default();
                if name.starts_with(TOOL_PREFIX) {
                    let args = msg["params"].get("arguments").cloned().unwrap_or_else(|| json!({}));
                    let id = id.unwrap_or(Value::Null);
                    return Some(match self.call(name, &args) {
                        Ok(r) => rpc_result(id, r.to_value()),
                        Err((code, m)) => rpc_error(id, code, m),
                    });
                }
                self.server.handle(msg)
            }
            "tools/list" => {
                let mut r = self.server.handle(msg)?;
                if let Some(list) = r.pointer_mut("/result/tools").and_then(Value::as_array_mut) {
                    let lc = std::mem::take(list);
                    list.extend(self.tools.iter().cloned());
                    list.extend(lc);
                }
                Some(r)
            }
            "initialize" => {
                let mut r = self.server.handle(msg)?;
                if let Some(res) = r.get_mut("result").and_then(Value::as_object_mut) {
                    let lc = res.get("instructions").and_then(Value::as_str).unwrap_or_default().to_string();
                    res.insert("serverInfo".into(), json!({"name": "br", "title": "BR", "version": env!("CARGO_PKG_VERSION")}));
                    res.insert("instructions".into(), json!(format!("{INSTRUCTIONS}\n\nLightCraft: {lc}")));
                }
                Some(r)
            }
            _ => self.server.handle(msg),
        }
    }

    /// Call one `br_*` tool. `Err` is a protocol error (unknown tool, bad argument names); a
    /// failing command is a tool result with `isError`.
    pub fn call(&mut self, name: &str, args: &Value) -> Result<ToolResult, (i64, String)> {
        let def = self.tools.iter().find(|t| t["name"] == name).ok_or((-32602, format!("unknown tool `{name}`")))?;
        let args = if args.is_null() { json!({}) } else { args.clone() };
        let Some(given) = args.as_object() else { return Err((-32602, "arguments must be an object".into())) };
        let props = def["inputSchema"]["properties"].as_object().cloned().unwrap_or_default();
        if let Some(k) = given.keys().find(|k| !props.contains_key(*k)) {
            let known: Vec<&String> = props.keys().collect();
            return Err((-32602, format!("unknown argument `{k}` for {name} (known: {known:?})")));
        }
        for r in def["inputSchema"]["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !given.contains_key(r) {
                return Err((-32602, format!("{name} needs `{r}`")));
            }
        }
        let backend = self.server.backend();
        let r = catch_unwind(AssertUnwindSafe(|| {
            if name == "br_preview" {
                let mut a = Map::new();
                if let Some(v) = given.get("id") {
                    a.insert("id".into(), v.clone());
                }
                a.insert("max_side".into(), given.get("max_side").cloned().unwrap_or(json!(1024)));
                a.insert("format".into(), given.get("format").cloned().unwrap_or(json!("jpeg")));
                return call_tool(backend, "render_preview", &Value::Object(a));
            }
            let Some(command) = command_of(name) else { return ToolResult::error(format!("no command for {name}")) };
            match backend.call("engine.execute", json!({"command": command, "params": args})) {
                Ok(v) => ToolResult::json(v),
                Err(e) => ToolResult::error(e),
            }
        }));
        Ok(r.unwrap_or_else(|_| ToolResult::error(format!("{name} failed unexpectedly (internal error); the session is still usable"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_command_and_a_valid_name() {
        for t in tools() {
            let name = t["name"].as_str().unwrap();
            assert!(name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{name}");
            if name != "br_preview" {
                assert!(command_of(name).is_some(), "{name}");
            }
        }
        // and every command has a tool
        for (id, _, _) in commands::COMMANDS {
            let name = format!("br_{}", id.trim_start_matches("br."));
            assert!(tools().iter().any(|t| t["name"] == name.as_str()), "{id}");
        }
    }
}
