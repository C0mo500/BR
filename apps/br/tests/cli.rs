//! The `br` binary end to end.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

fn br() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_br"));
    c.env("LIGHTCRAFT_GPU", "0");
    c
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("br-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn lines(out: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(out).lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[test]
fn help_and_version() {
    let o = br().arg("--help").output().unwrap();
    assert!(o.status.success());
    let text = String::from_utf8_lossy(&o.stdout);
    for s in ["br mcp", "br photo", "br run", "br snapshot"] {
        assert!(text.contains(s), "{s}");
    }
    let o = br().args(["photo", "--nope"]).output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("unknown option"));
}

#[test]
fn snapshot_then_edit_that_picture_with_br_photo() {
    let dir = temp("photo");
    let shot = dir.join("shot.png");
    let o = br().args(["snapshot", "--demo", "--tab", "format", "--size", "1000x700", "-o"]).arg(&shot).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(std::fs::metadata(&shot).unwrap().len() > 10_000);

    // the screenshot is a photo like any other: a look, two sliders, a story export
    let out = dir.join("story.jpg");
    let o = br()
        .arg("photo")
        .arg(&shot)
        .args(["--look", "lc.cinematic:70", "--light", "10", "--wärme", "-15", "--preset", "story", "-o"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let r = &lines(&o.stdout)[0];
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(1080), Some(1920)), "{r}");
    assert!(out.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_prints_one_json_line_per_command() {
    let o = br()
        .args(["run", "--demo", "br.look", "look=lc.moody", "amount=50", "br.adjust", "contrast=20", "br.crop", "format=1:1", "br.state"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let l = lines(&o.stdout);
    assert_eq!(l.len(), 4);
    assert!(l.iter().all(|x| x["ok"] == true));
    assert_eq!(l[3]["result"]["format"], "1:1");
    assert_eq!(l[3]["result"]["sliders"]["contrast"], 20.0);

    // a failing command stops the run and fails it
    let o = br().args(["run", "--demo", "br.crop", "format=3:2", "br.state"]).output().unwrap();
    assert!(!o.status.success());
    let l = lines(&o.stdout);
    assert_eq!(l.len(), 1);
    assert_eq!(l[0]["ok"], false);
}

#[test]
fn mcp_over_stdio() {
    let mut child = br().args(["mcp", "--demo"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for m in [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"br_look","arguments":{"look":"lc.vivid-pop"}}}"#,
        ] {
            writeln!(stdin, "{m}").unwrap();
        }
    }
    drop(child.stdin.take());
    let o = child.wait_with_output().unwrap();
    let l = lines(&o.stdout);
    assert_eq!(l.len(), 3, "{l:?}");
    assert_eq!(l[0]["result"]["serverInfo"]["name"], "br");
    assert!(l[1]["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "br_export"));
    assert_eq!(l[2]["result"]["isError"], false, "{}", l[2]);
}
