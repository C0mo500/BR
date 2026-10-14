//! `br mcp` end to end: JSON-RPC lines in, replies out, on LightCraft's demo photos.

use br_mcp::{BrBackend, BrServer};
use lightcraft_mcp::Headless;
use serde_json::{Value, json};

fn server(full: bool) -> BrServer {
    lightcraft_engine::gpu::set_enabled(false);
    let mut s = BrServer::new(BrBackend::new(Headless::demo(), "demo photos"), full);
    let init = s.handle(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}));
    assert_eq!(init.unwrap()["result"]["serverInfo"]["name"], "br");
    assert!(s.handle(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
    s
}

fn call(s: &mut BrServer, name: &str, args: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string();
    let reply: Value = serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap();
    assert_eq!(reply["id"], 7);
    reply
}

fn ok(s: &mut BrServer, name: &str, args: Value) -> Value {
    let r = call(s, name, args);
    assert_eq!(r["result"]["isError"], false, "{name}: {r}");
    let text = r["result"]["content"][0]["text"].as_str().unwrap_or("null");
    serde_json::from_str(text).unwrap_or(Value::Null)
}

#[test]
fn br_tools_come_first_and_lightcrafts_follow() {
    let mut s = server(false);
    let r = s.handle(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
    let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names[0], "br_state");
    for n in ["br_look", "br_adjust", "br_crop", "br_export", "br_preview", "import", "set_develop", "command_run"] {
        assert!(names.contains(&n), "{n} missing from {names:?}");
    }
    assert!(!names.iter().any(|n| n.starts_with("cmd_")), "compact by default");
    let full = server(true).handle(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
    let full: Vec<&str> = full["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(full.contains(&"cmd_br_look") && full.contains(&"cmd_develop_set"), "full mode lists every command");
}

#[test]
fn an_agent_edits_and_exports_a_photo() {
    let mut s = server(false);
    let looks = ok(&mut s, "br_looks", json!({}));
    assert!(looks.as_array().unwrap().len() >= 18);
    ok(&mut s, "br_look", json!({"look": "lc.teal-orange", "amount": 80}));
    let r = ok(&mut s, "br_adjust", json!({"light": 25, "warmth": 15}));
    assert_eq!(r["sliders"]["light"], 25.0);
    let r = ok(&mut s, "br_crop", json!({"format": "9:16"}));
    assert_eq!(r["format"], "9:16");
    let st = ok(&mut s, "br_state", json!({}));
    assert_eq!(st["format"], "9:16");
    assert_eq!(st["sliders"]["warmth"], 15.0);
    assert_eq!(st["undo"], "Format 9:16");

    // a preview comes back as an image
    let r = call(&mut s, "br_preview", json!({"max_side": 256}));
    let content = r["result"]["content"].as_array().unwrap();
    assert!(content.iter().any(|c| c["type"] == "image" && c["mimeType"] == "image/jpeg"), "{r}");

    let dir = std::env::temp_dir().join(format!("br-mcp-e2e-{}", std::process::id()));
    let r = ok(&mut s, "br_export", json!({"preset": "story", "dir": dir.to_string_lossy()}));
    let f = &r["files"][0];
    assert_eq!((f["width"].as_u64(), f["height"].as_u64()), (Some(1080), Some(1920)), "{r}");
    assert!(std::path::Path::new(f["path"].as_str().unwrap()).exists());
    let _ = std::fs::remove_dir_all(&dir);

    // undo through BR, and the same command through LightCraft's generic tool
    ok(&mut s, "br_undo", json!({}));
    assert_eq!(ok(&mut s, "br_state", json!({}))["undo"], "Anpassen");
    let r = ok(&mut s, "command_run", json!({"id": "br.state", "params": {}}));
    assert_eq!(r["sliders"]["light"], 25.0);
}

#[test]
fn mistakes_are_reported_clearly() {
    let mut s = server(false);
    // a wrong argument name is a protocol error
    let r = call(&mut s, "br_adjust", json!({"brightness": 10}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = call(&mut s, "br_crop", json!({}));
    assert!(r["error"]["message"].as_str().unwrap().contains("format"));
    // a failing command is a tool error, and the server keeps going
    let r = call(&mut s, "br_look", json!({"look": "nope"}));
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("unknown look"));
    let r = call(&mut s, "br_nope", json!({}));
    assert!(r["error"]["message"].as_str().unwrap().contains("unknown tool"));
    ok(&mut s, "br_state", json!({}));
}
