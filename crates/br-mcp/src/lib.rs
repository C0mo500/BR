//! `br mcp`: BR Foto for AI agents over the Model Context Protocol.
//!
//! LightCraft already ships a complete MCP server (`lightcraft-mcp`): import, query, develop,
//! render and export tools, plus one `cmd_*` tool per engine command. BR keeps all of it and adds:
//!
//! - [`BrBackend`]: LightCraft's in-process [`Headless`] backend with the `br.*` commands of
//!   [`br_photo::commands`] routed in, so they also work through `command_run` and, with `--full`,
//!   as `cmd_br_*` tools.
//! - A front ([`serve`], [`BrServer`]) for LightCraft's [`Server`]: it names the server `br`, puts
//!   BR's instructions first and lists typed `br_*` tools (proper schemas, a preview image) ahead
//!   of LightCraft's own. A `br_*` call is checked against its schema and handed to LightCraft's
//!   server as the matching `command_run` (or `render_preview`) call, so everything, BR's tools
//!   included, runs through LightCraft's own request handling: id checks, protocol versions,
//!   progress notifications, cancellation and `ping` during long exports.
//!
//! The protocol is newline-delimited JSON-RPC 2.0 over stdio.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};

use br_photo::BrState;
use br_photo::adjust::Slider;
use br_photo::commands;
use br_photo::crop::Format;
use br_photo::export::Preset;
use lightcraft_mcp::{Backend, Headless, ProgressHook, Server};
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
    // edits replace what the photo had (undoable, but a client asking before "destructive" calls
    // should ask); an export only writes a new file
    let destructive = !read_only && name != "br_export";
    let idempotent = read_only || matches!(name, "br_adjust" | "br_crop" | "br_reset");
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false},
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": destructive, "idempotentHint": idempotent, "openWorldHint": false},
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
            "Apply a look (a filter preset) to a photo. If the photo's latest edit is a look, it is replaced (the same look at a new \
             amount, or another look), so looks never stack and stay one undo step. Amount 0 takes the look off.",
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

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// Check `args` against a tool's input schema: known names, required ones, types, enums, ranges.
fn check_args(def: &Value, args: &Value) -> Result<(), String> {
    let name = def["name"].as_str().unwrap_or_default();
    let Some(given) = args.as_object() else { return Err("arguments must be an object".into()) };
    let props = def["inputSchema"]["properties"].as_object().cloned().unwrap_or_default();
    for (k, v) in given {
        let Some(schema) = props.get(k) else {
            let known: Vec<&String> = props.keys().collect();
            return Err(format!("unknown argument `{k}` for {name} (known: {known:?})"));
        };
        // an optional argument sent as null (strict tool-calling modes do) is not given
        if v.is_null() {
            continue;
        }
        let ok = match schema["type"].as_str() {
            Some("integer") => v.is_u64() || v.is_i64(),
            Some("number") => v.as_f64().is_some_and(f64::is_finite),
            Some("string") => v.is_string(),
            _ => true,
        };
        if !ok {
            let want = match schema["type"].as_str() {
                Some("integer") => "a whole number",
                Some("number") => "a number",
                _ => "text",
            };
            return Err(format!("{name}: `{k}` must be {want} (got {v})"));
        }
        if let Some(options) = schema["enum"].as_array()
            && !options.contains(v)
        {
            return Err(format!("{name}: `{k}` must be one of {}", Value::Array(options.clone())));
        }
        if let Some(x) = v.as_f64()
            && (schema["minimum"].as_f64().is_some_and(|m| x < m) || schema["maximum"].as_f64().is_some_and(|m| x > m))
        {
            return Err(format!("{name}: `{k}` must be from {} to {}", schema["minimum"], schema["maximum"]));
        }
    }
    for r in def["inputSchema"]["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if given.get(r).is_none_or(Value::is_null) {
            return Err(format!("{name} needs `{r}`"));
        }
    }
    Ok(())
}

/// What the front does with one message.
enum Front {
    /// Answer it here (a bad `br_*` call), without LightCraft.
    Reply(Value),
    /// Hand it (possibly rewritten) to LightCraft's server; patch the reply to `id` if asked.
    Forward(Value, Option<(String, Patch)>),
}

/// A change to one of LightCraft's replies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Patch {
    Initialize,
    ToolsList,
}

fn valid_id(id: Option<&Value>) -> Option<&Value> {
    id.filter(|i| i.is_string() || i.is_number())
}

fn front(tools: &[Value], msg: Value) -> Front {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    let Some(id) = valid_id(msg.get("id")).cloned() else { return Front::Forward(msg, None) };
    match method {
        "initialize" => Front::Forward(msg, Some((id.to_string(), Patch::Initialize))),
        "tools/list" => Front::Forward(msg, Some((id.to_string(), Patch::ToolsList))),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or_default().to_string();
            if !name.starts_with(TOOL_PREFIX) || !msg["params"].is_object() {
                return Front::Forward(msg, None);
            }
            let Some(def) = tools.iter().find(|t| t["name"] == name.as_str()) else {
                return Front::Reply(rpc_error(id, -32602, format!("unknown tool `{name}`")));
            };
            let mut args = match msg["params"].get("arguments") {
                None | Some(Value::Null) => json!({}),
                Some(a) => a.clone(),
            };
            if let Err(e) = check_args(def, &args) {
                return Front::Reply(rpc_error(id, -32602, e));
            }
            if let Some(o) = args.as_object_mut() {
                o.retain(|_, v| !v.is_null());
            }
            let (call, call_args) = if name == "br_preview" {
                let mut a = Map::new();
                if let Some(v) = args.get("id") {
                    a.insert("id".into(), v.clone());
                }
                a.insert("max_side".into(), args.get("max_side").cloned().unwrap_or(json!(1024)));
                a.insert("format".into(), args.get("format").cloned().unwrap_or(json!("jpeg")));
                ("render_preview", Value::Object(a))
            } else {
                let Some(command) = command_of(&name) else { return Front::Reply(rpc_error(id, -32602, format!("unknown tool `{name}`"))) };
                ("command_run", json!({"id": command, "params": args}))
            };
            let mut msg = msg;
            msg["params"]["name"] = json!(call);
            msg["params"]["arguments"] = call_args;
            Front::Forward(msg, None)
        }
        _ => Front::Forward(msg, None),
    }
}

/// Apply `patch` to LightCraft's reply `r`.
fn patch(tools: &[Value], patch: Patch, r: &mut Value) {
    let Some(res) = r.get_mut("result").and_then(Value::as_object_mut) else { return };
    match patch {
        Patch::ToolsList => {
            if let Some(list) = res.get_mut("tools").and_then(Value::as_array_mut) {
                let lc = std::mem::take(list);
                list.extend(tools.iter().cloned());
                list.extend(lc);
            }
        }
        Patch::Initialize => {
            let lc = res.get("instructions").and_then(Value::as_str).unwrap_or_default().to_string();
            res.insert("serverInfo".into(), json!({"name": "br", "title": "BR", "version": env!("CARGO_PKG_VERSION")}));
            res.insert("instructions".into(), json!(format!("{INSTRUCTIONS}\n\nLightCraft: {lc}")));
        }
    }
}

/// Patch every reply in `v` (one reply or a batch) whose id has a pending patch.
fn patch_reply(tools: &[Value], pending: &Mutex<HashMap<String, Patch>>, v: &mut Value) {
    let one = |r: &mut Value| {
        let Some(id) = valid_id(r.get("id")).map(Value::to_string) else { return };
        let p = pending.lock().ok().and_then(|mut m| m.remove(&id));
        if let Some(p) = p {
            patch(tools, p, r);
        }
    };
    match v {
        Value::Array(a) => a.iter_mut().for_each(one),
        r => one(r),
    }
}

/// LightCraft's MCP server with BR in front, called message by message (tests, embedding). For
/// stdio use [`serve`], which also keeps progress notifications and cancellation.
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

    /// Handle one line; the reply line, or `None` for notifications and blank lines.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) if !batch.is_empty() => {
                let replies: Vec<Value> = batch.into_iter().filter_map(|m| self.handle(m)).collect();
                (!replies.is_empty()).then(|| Value::Array(replies).to_string())
            }
            Ok(msg @ Value::Object(_)) => self.handle(msg).map(|r| r.to_string()),
            // anything else (an empty batch, a bad message, bad JSON) LightCraft's server answers
            _ => self.server.handle_line(line),
        }
    }

    /// Handle one JSON-RPC message.
    pub fn handle(&mut self, msg: Value) -> Option<Value> {
        match front(&self.tools, msg) {
            Front::Reply(r) => Some(r),
            Front::Forward(msg, p) => {
                let mut r = self.server.handle(msg)?;
                if let Some((_, p)) = p {
                    patch(&self.tools, p, &mut r);
                }
                Some(r)
            }
        }
    }
}

/// Serve newline-delimited JSON-RPC from `input` to `output` until `input` closes.
///
/// LightCraft's own server loop runs on a thread of its own and does all the work; this thread
/// reads the client's lines, rewrites `br_*` calls (or answers bad ones), and a third thread
/// writes LightCraft's replies out, patching `initialize` and `tools/list`. Notifications such as
/// progress pass straight through, and a cancellation reaches LightCraft while it exports.
pub fn serve(backend: BrBackend, full: bool, input: impl BufRead, output: impl Write + Send + 'static) -> std::io::Result<()> {
    let tools = Arc::new(tools());
    let out = Arc::new(Mutex::new(output));
    let pending: Arc<Mutex<HashMap<String, Patch>>> = Arc::default();
    // answers BR gave itself inside a batch, by the id of a request LightCraft answers in it
    let extras: Arc<Mutex<HashMap<String, Vec<Value>>>> = Arc::default();
    let (lc_in, mut to_lc) = std::io::pipe()?;
    let (from_lc, lc_out) = std::io::pipe()?;
    // LightCraft's server is not `Send`: it is built on the thread that runs it
    let server = std::thread::Builder::new()
        .name("br-mcp-lightcraft".into())
        .spawn(move || Server::new(Box::new(backend)).with_command_tools(full).serve(BufReader::new(lc_in), lc_out))?;
    let pump = {
        let (tools, out, pending, extras) = (tools.clone(), out.clone(), pending.clone(), extras.clone());
        std::thread::Builder::new().name("br-mcp-out".into()).spawn(move || -> std::io::Result<()> {
            for line in BufReader::new(from_lc).lines() {
                let line = line?;
                let text = match serde_json::from_str::<Value>(&line) {
                    Ok(mut v) => {
                        patch_reply(&tools, &pending, &mut v);
                        if let Value::Array(a) = &mut v {
                            let ids: Vec<String> = a.iter().filter_map(|r| valid_id(r.get("id")).map(Value::to_string)).collect();
                            if let Ok(mut map) = extras.lock() {
                                for id in ids {
                                    if let Some(more) = map.remove(&id) {
                                        a.extend(more);
                                    }
                                }
                            }
                        }
                        v.to_string()
                    }
                    Err(_) => line,
                };
                let mut o = out.lock().map_err(|_| std::io::Error::other("output lock poisoned"))?;
                writeln!(o, "{text}")?;
                o.flush()?;
            }
            Ok(())
        })?
    };
    let reply = |v: &Value| -> std::io::Result<()> {
        let mut o = out.lock().map_err(|_| std::io::Error::other("output lock poisoned"))?;
        writeln!(o, "{v}")?;
        o.flush()
    };
    let mut result = Ok(());
    for line in input.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                result = Err(e);
                break;
            }
        };
        let forward = match serde_json::from_str::<Value>(line.trim()) {
            Ok(Value::Array(batch)) if !batch.is_empty() => {
                let (mut keep, mut rejected) = (Vec::new(), Vec::new());
                for m in batch {
                    match front(&tools, m) {
                        Front::Reply(r) => rejected.push(r),
                        Front::Forward(m, p) => {
                            if let (Some((id, p)), Ok(mut map)) = (p, pending.lock()) {
                                map.insert(id, p);
                            }
                            keep.push(m);
                        }
                    }
                }
                // the batch is answered as one array: BR's own answers join LightCraft's
                match keep.iter().find_map(|m| valid_id(m.get("id")).filter(|_| m.get("method").is_some())) {
                    Some(id) if !rejected.is_empty() => {
                        if let Ok(mut map) = extras.lock() {
                            map.insert(id.to_string(), rejected);
                        }
                    }
                    _ if !rejected.is_empty() => reply(&Value::Array(rejected))?,
                    _ => {}
                }
                (!keep.is_empty()).then(|| Value::Array(keep).to_string())
            }
            Ok(msg @ Value::Object(_)) => match front(&tools, msg) {
                Front::Reply(r) => {
                    reply(&r)?;
                    None
                }
                Front::Forward(m, p) => {
                    if let (Some((id, p)), Ok(mut map)) = (p, pending.lock()) {
                        map.insert(id, p);
                    }
                    Some(m.to_string())
                }
            },
            // blank lines, bad JSON, empty batches: LightCraft answers them as it would
            _ => Some(line),
        };
        if let Some(f) = forward
            && (writeln!(to_lc, "{f}").is_err() || to_lc.flush().is_err())
        {
            break; // LightCraft's server has stopped
        }
    }
    drop(to_lc); // end of input for LightCraft's server
    let served = server.join().map_err(|_| std::io::Error::other("the MCP server thread panicked"))?;
    let pumped = pump.join().map_err(|_| std::io::Error::other("the MCP output thread panicked"))?;
    result.and(served).and(pumped)
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

    #[test]
    fn edits_are_destructive_reads_and_exports_are_not() {
        for t in tools() {
            let a = &t["annotations"];
            let name = t["name"].as_str().unwrap();
            let expect = a["readOnlyHint"] == false && name != "br_export";
            assert_eq!(a["destructiveHint"], expect, "{name}");
        }
    }

    #[test]
    fn arguments_are_checked_against_the_schema() {
        let tools = tools();
        let def = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap().clone();
        assert!(check_args(&def("br_adjust"), &json!({"light": 10, "id": 3})).is_ok());
        for (tool, args) in [
            ("br_adjust", json!({"brightness": 1})),
            ("br_adjust", json!({"light": 101})),
            ("br_adjust", json!({"light": "10"})),
            ("br_state", json!({"id": "14"})),
            ("br_state", json!({"id": 14.5})),
            ("br_preview", json!({"id": "3"})),
            ("br_crop", json!({"format": "3:2"})),
            ("br_crop", json!({})),
            ("br_look", json!({"look": "lc.moody", "amount": -1})),
        ] {
            assert!(check_args(&def(tool), &args).is_err(), "{tool} {args}");
        }
    }
}
