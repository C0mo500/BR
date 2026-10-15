//! The `shnyp` binary end to end.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

fn shnyp() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_shnyp"));
    c.env("LIGHTCRAFT_GPU", "0");
    c
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("shnyp-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn lines(out: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(out).lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[test]
fn help_and_version() {
    let o = shnyp().arg("--help").output().unwrap();
    assert!(o.status.success());
    let text = String::from_utf8_lossy(&o.stdout);
    for s in ["shnyp mcp", "shnyp photo", "shnyp run", "shnyp snapshot"] {
        assert!(text.contains(s), "{s}");
    }
    let o = shnyp().args(["photo", "--nope"]).output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("unknown option"));
}

#[test]
fn snapshot_then_edit_that_picture_with_shnyp_photo() {
    let dir = temp("photo");
    let shot = dir.join("shot.png");
    let o = shnyp().args(["snapshot", "--demo", "--popover", "format", "--size", "1000x700", "-o"]).arg(&shot).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(std::fs::metadata(&shot).unwrap().len() > 10_000);

    // the screenshot is a photo like any other: a look, two sliders, a story export
    let out = dir.join("story.jpg");
    let o = shnyp()
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
fn snapshot_draws_any_page_and_names_the_pages_it_knows() {
    let dir = temp("pages");
    let shot = dir.join("farbe.png");
    let o = shnyp().args(["snapshot", "--page", "farbe", "--size", "1000x700", "-o"]).arg(&shot).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(std::fs::metadata(&shot).unwrap().len() > 10_000);
    let o = shnyp().args(["snapshot", "--page", "fusion"]).output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("medien, schnitt, farbe, ton, ausgabe, foto"));
    let o = shnyp().args(["snapshot", "--page", "ton", "--popover", "looks"]).output().unwrap();
    assert!(!o.status.success(), "the photo tools are not on the Ton page");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_prints_one_json_line_per_command() {
    let o = shnyp()
        .args(["run", "--demo", "shnyp.look", "look=lc.moody", "amount=50", "shnyp.adjust", "contrast=20", "shnyp.crop", "format=1:1", "shnyp.state"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let l = lines(&o.stdout);
    assert_eq!(l.len(), 4);
    assert!(l.iter().all(|x| x["ok"] == true));
    assert_eq!(l[3]["result"]["format"], "1:1");
    assert_eq!(l[3]["result"]["sliders"]["contrast"], 20.0);

    // a failing command stops the run and fails it
    let o = shnyp().args(["run", "--demo", "shnyp.crop", "format=3:2", "shnyp.state"]).output().unwrap();
    assert!(!o.status.success());
    let l = lines(&o.stdout);
    assert_eq!(l.len(), 1);
    assert_eq!(l[0]["ok"], false);
}

#[test]
fn mcp_over_stdio() {
    let mut child = shnyp().args(["mcp", "--demo"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for m in [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"shnyp_look","arguments":{"look":"lc.vivid-pop"}}}"#,
        ] {
            writeln!(stdin, "{m}").unwrap();
        }
    }
    drop(child.stdin.take());
    let o = child.wait_with_output().unwrap();
    let l = lines(&o.stdout);
    assert_eq!(l.len(), 3, "{l:?}");
    assert_eq!(l[0]["result"]["serverInfo"]["name"], "shnyp");
    assert!(l[1]["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "shnyp_export"));
    assert_eq!(l[2]["result"]["isError"], false, "{}", l[2]);
}

/// A small picture to import: a snapshot of the window. Different names give different pictures
/// (LightCraft recognises the same picture under another name as a duplicate).
fn picture(dir: &std::path::Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    let width = 400 + 10 * name.len();
    let o = shnyp().args(["snapshot", "--demo", "--size", &format!("{width}x300"), "-o"]).arg(&p).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    p
}

#[test]
fn run_import_works_on_the_photo_it_names_even_when_it_is_already_there() {
    let dir = temp("import");
    let lib = dir.join("lib");
    let (a, b) = (picture(&dir, "a.png"), picture(&dir, "bb.png"));
    let state = |path: &PathBuf| {
        let o = shnyp().arg("run").arg(format!("--library={}", lib.display())).arg("--import").arg(path).arg("shnyp.state").output().unwrap();
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
    let o = shnyp().args(["run", "--library=", "shnyp.looks"]).current_dir(&dir).output().unwrap();
    assert!(!o.status.success());
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none(), "nothing written in the current folder");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_snapshot_sizes_and_photo_outputs_are_refused() {
    for size in ["0x0", "100000x100000"] {
        let o = shnyp().args(["snapshot", "--demo", "--size", size, "-o", "/tmp/never.png"]).output().unwrap();
        assert!(!o.status.success(), "{size}");
    }
    let dir = temp("pngout");
    let pic = picture(&dir, "in.png");
    let out = dir.join("out.png");
    let o = shnyp().arg("photo").arg(&pic).arg("-o").arg(&out).output().unwrap();
    assert!(!o.status.success());
    assert!(!out.exists(), "no folder named out.png");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_numeric_folder_name_stays_a_folder_name() {
    let dir = temp("numdir");
    let o = shnyp()
        .args(["run", "--demo", "shnyp.export", "preset=square", "dir=2024"])
        .current_dir(&dir)
        .env("SHNYP_EXPORTS", dir.join("elsewhere"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(dir.join("2024").is_dir());
    assert!(!dir.join("elsewhere").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_closed_pipe_ends_quietly_and_groups_are_listed_once() {
    let mut child = shnyp().arg("looks").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut first = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(child.stdout.take().unwrap()), &mut first).unwrap();
    let o = child.wait_with_output().unwrap();
    assert_ne!(o.status.code(), Some(101), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!String::from_utf8_lossy(&o.stderr).contains("panicked"));
    let o = shnyp().arg("looks").output().unwrap();
    let headings: Vec<String> = String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with(' ')).map(String::from).collect();
    let mut unique = headings.clone();
    unique.dedup();
    unique.sort();
    unique.dedup();
    assert_eq!(headings.len(), unique.len(), "{headings:?}");
}

#[test]
fn a_failed_import_stops_before_any_command() {
    let o = shnyp().args(["run", "--demo", "--import", "/nonexistent/typo.jpg", "shnyp.state"]).output().unwrap();
    assert!(!o.status.success());
    assert!(lines(&o.stdout).is_empty(), "no command ran on another photo");
    assert!(String::from_utf8_lossy(&o.stderr).contains("could not import"));
}

/// Only the window keeps LightCraft's settings and a log file: the command line, `shnyp snapshot`
/// and a `--demo` window write nothing into shnyp's folders (`SHNYP_CONFIG_DIR` puts them in `profile`).
#[test]
fn only_the_window_keeps_settings_and_a_log() {
    let dir = temp("profile");
    let profile = dir.join("profile");
    let shot = dir.join("shot.png");
    let quiet = |c: &mut Command| c.env("SHNYP_CONFIG_DIR", &profile).output().unwrap();
    assert!(quiet(shnyp().args(["run", "--demo", "shnyp.look", "look=lc.moody", "shnyp.state"])).status.success());
    assert!(quiet(shnyp().args(["snapshot", "--demo", "--size", "500x300", "-o"]).arg(&shot)).status.success());
    assert!(quiet(shnyp().arg("photo").arg(&shot).arg("-o").arg(dir.join("out.jpg"))).status.success());
    assert!(quiet(shnyp().arg("looks")).status.success());
    assert!(!profile.exists(), "the command line wrote into shnyp's folders");
    // the window, without a display: it can't open, but has started its log by then
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let window = |args: &[&str]| {
            let o = quiet(shnyp().args(args).env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY").env_remove("WAYLAND_SOCKET"));
            assert!(!o.status.success());
            assert!(String::from_utf8_lossy(&o.stderr).contains("could not open the window"), "{}", String::from_utf8_lossy(&o.stderr));
        };
        window(&["--demo"]);
        assert!(!profile.exists(), "a --demo window wrote into shnyp's folders");
        let lib = dir.join("lib");
        window(&["--library", &lib.to_string_lossy()]);
        let log = std::fs::read_to_string(profile.join("Logs").join("shnyp.log")).unwrap();
        assert!(log.contains("log file") && log.contains("could not open the window"), "{log}");
        window(&["--library", &lib.to_string_lossy()]);
        assert!(profile.join("Logs").join("shnyp.1.log").exists(), "the previous log is kept");
        // nothing changed in the window, so no settings file
        assert!(!profile.join("lightcraft-ui.json").exists());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn have_ffmpeg() -> bool {
    let found = Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());
    // in CI (SHNYP_REQUIRE_FFMPEG) the video tests must run, not skip
    assert!(found || std::env::var_os("SHNYP_REQUIRE_FFMPEG").is_none(), "ffmpeg is required (SHNYP_REQUIRE_FFMPEG is set)");
    found
}

fn phone_clip(path: &std::path::Path, seconds: u32) {
    let o = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size=1080x1920:rate=30:duration={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=440:duration={seconds}"))
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(path)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn run_edits_a_video_with_shnyps_and_filmcrafts_commands() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg missing: skipped");
        return;
    }
    let dir = temp("video");
    let clip = dir.join("IMG_0001.mp4");
    phone_clip(&clip, 3);
    let out = dir.join("reel.mp4");
    let o = shnyp()
        .arg("run")
        .arg("--import")
        .arg(&clip)
        .args([
            "shnyp.video.split",
            "seconds=1",
            "film.sequence.inspect",
            "shnyp.video.text",
            "text=Hallo",
            "shnyp.video.export",
            "preset=social",
            "wait=true",
        ])
        .arg(format!("path={}", out.display()))
        .arg("shnyp.video.state")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let l = lines(&o.stdout);
    assert_eq!(l.len(), 5, "{l:?}");
    assert!(l.iter().all(|x| x["ok"] == true), "{l:?}");
    assert_eq!(l[1]["result"]["video"][0]["items"].as_array().map(Vec::len), Some(2), "split in two: {}", l[1]);
    assert_eq!(l[4]["result"]["format"], "9:16");
    assert!(std::fs::metadata(&out).unwrap().len() > 1000);

    // the window's snapshot with that clip opens the Schnitt page on it
    let shot = dir.join("schnitt.png");
    let o = shnyp().arg("snapshot").arg(&clip).args(["--size", "1000x700", "-o"]).arg(&shot).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(std::fs::metadata(&shot).unwrap().len() > 10_000);
    let o = shnyp().args(["snapshot", "/nowhere/gone.mp4"]).output().unwrap();
    assert!(!o.status.success(), "a video that is not there is an error");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn commands_lists_photo_video_and_filmcrafts() {
    let o = shnyp().arg("commands").output().unwrap();
    assert!(o.status.success());
    let text = String::from_utf8_lossy(&o.stdout);
    for c in ["shnyp.look", "shnyp.video.add", "shnyp.video.autoCaptions", "shnyp.video.format", "film.<id>"] {
        assert!(text.contains(c), "{c} missing:\n{text}");
    }
    let o = shnyp().args(["commands", "--film"]).output().unwrap();
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("film.timeline.place") && text.contains("film.export.quick"), "{text}");
}

#[test]
fn run_reads_a_script_and_shnyp_film_runs_filmcraft_s_commands() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg missing: skipped");
        return;
    }
    let dir = temp("script");
    let clip = dir.join("a.mp4");
    phone_clip(&clip, 3);
    let script = dir.join("schnitt.jsonl");
    std::fs::write(
        &script,
        "# zwei Schritte\n{\"command\": \"shnyp.video.split\", \"params\": {\"seconds\": 1}}\n\n{\"command\": \"shnyp.video.anchors\"}\n",
    )
    .unwrap();
    let o = shnyp().arg("run").arg("--import").arg(&clip).arg("--script").arg(&script).arg("shnyp.video.state").output().unwrap();
    assert!(o.status.success(), "{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let l = lines(&o.stdout);
    let ids: Vec<&str> = l.iter().filter_map(|x| x["command"].as_str()).collect();
    assert_eq!(ids, vec!["shnyp.video.state", "shnyp.video.split", "shnyp.video.anchors"], "command line first, then the script: {l:?}");
    assert_eq!(l[2]["result"]["magnet"], true, "{}", l[2]);

    // from standard input
    let mut child = shnyp().args(["run", "--script", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"{\"command\": \"shnyp.video.new\", \"params\": {\"format\": \"1:1\"}}\n").unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success());
    assert_eq!(lines(&o.stdout)[0]["result"]["format"], "1:1");

    // a broken line names its place
    std::fs::write(&script, "{\"command\": \"shnyp.video.state\"}\nkein json\n").unwrap();
    let o = shnyp().arg("run").arg("--script").arg(&script).output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("schnitt.jsonl:2"), "{}", String::from_utf8_lossy(&o.stderr));

    // shnyp film: FilmCraft's ids without the prefix
    let o = shnyp().arg("film").arg("--import").arg(&clip).args(["sequence.inspect", "timeline.razor", "time=0"]).output().unwrap();
    let l = lines(&o.stdout);
    assert_eq!(l[0]["command"], "film.sequence.inspect", "{l:?}");
    assert!(l[0]["result"]["video"].is_array(), "{l:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
