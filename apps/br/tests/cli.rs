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

/// A small picture to import: a snapshot of the window. Different names give different pictures
/// (LightCraft recognises the same picture under another name as a duplicate).
fn picture(dir: &std::path::Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    let width = 400 + 10 * name.len();
    let o = br().args(["snapshot", "--demo", "--size", &format!("{width}x300"), "-o"]).arg(&p).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    p
}

#[test]
fn run_import_works_on_the_photo_it_names_even_when_it_is_already_there() {
    let dir = temp("import");
    let lib = dir.join("lib");
    let (a, b) = (picture(&dir, "a.png"), picture(&dir, "bb.png"));
    let state = |path: &PathBuf| {
        let o = br().arg("run").arg(format!("--library={}", lib.display())).arg("--import").arg(path).arg("br.state").output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        lines(&o.stdout)[0]["result"].clone()
    };
    let first = state(&a);
    assert_eq!(first["file"], "a.png");
    assert_eq!(state(&b)["file"], "bb.png");
    // a.png again: already in the library, and still the photo the commands work on
    let again = state(&a);
    assert_eq!((again["file"].as_str(), again["id"].as_u64()), (Some("a.png"), first["id"].as_u64()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_library_option_is_refused() {
    let dir = temp("emptylib");
    let o = br().args(["run", "--library=", "br.looks"]).current_dir(&dir).output().unwrap();
    assert!(!o.status.success());
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none(), "nothing written in the current folder");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_snapshot_sizes_and_photo_outputs_are_refused() {
    for size in ["0x0", "100000x100000"] {
        let o = br().args(["snapshot", "--demo", "--size", size, "-o", "/tmp/never.png"]).output().unwrap();
        assert!(!o.status.success(), "{size}");
    }
    let dir = temp("pngout");
    let pic = picture(&dir, "in.png");
    let out = dir.join("out.png");
    let o = br().arg("photo").arg(&pic).arg("-o").arg(&out).output().unwrap();
    assert!(!o.status.success());
    assert!(!out.exists(), "no folder named out.png");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_numeric_folder_name_stays_a_folder_name() {
    let dir = temp("numdir");
    let o = br()
        .args(["run", "--demo", "br.export", "preset=square", "dir=2024"])
        .current_dir(&dir)
        .env("BR_EXPORTS", dir.join("elsewhere"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(dir.join("2024").is_dir());
    assert!(!dir.join("elsewhere").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_closed_pipe_ends_quietly_and_groups_are_listed_once() {
    let mut child = br().arg("looks").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut first = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(child.stdout.take().unwrap()), &mut first).unwrap();
    let o = child.wait_with_output().unwrap();
    assert_ne!(o.status.code(), Some(101), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!String::from_utf8_lossy(&o.stderr).contains("panicked"));
    let o = br().arg("looks").output().unwrap();
    let headings: Vec<String> = String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with(' ')).map(String::from).collect();
    let mut unique = headings.clone();
    unique.dedup();
    unique.sort();
    unique.dedup();
    assert_eq!(headings.len(), unique.len(), "{headings:?}");
}

#[test]
fn a_failed_import_stops_before_any_command() {
    let o = br().args(["run", "--demo", "--import", "/nonexistent/typo.jpg", "br.state"]).output().unwrap();
    assert!(!o.status.success());
    assert!(lines(&o.stdout).is_empty(), "no command ran on another photo");
    assert!(String::from_utf8_lossy(&o.stderr).contains("could not import"));
}
