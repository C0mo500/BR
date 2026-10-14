//! `br`: BR's window and command line.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use br_mcp::BrBackend;
use br_photo::adjust::Slider;
use br_photo::crop::Format;
use br_photo::export::{Preset, Target};
use br_photo::{commands, library};
use lightcraft_engine::Session;
use lightcraft_mcp::{Backend, Headless, expand_paths};
use serde_json::{Value, json};

const USAGE: &str = "\
BR — eine Media-Suite: DaVinci-Umfang, so direkt bedienbar wie CapCut. Seiten: Foto (LightCraft).

USAGE
  br [--library DIR | --demo] [FILES…]
      Open the window. FILES (photos or folders) are imported. The library is DIR, else
      $BR_LIBRARY, else ~/Pictures/BR Library; --demo opens LightCraft's demo photos in memory.
      BR_RENDERER=glow or =wgpu picks the window renderer; BR_WAYLAND=1 keeps native Wayland
      (by default BR uses X11 where it can, which is what makes dropping files work).
      BR_IMPORT_DIR is the folder the import dialog opens in (see deploy/browser for a BR
      on a server, used in the browser).

  br mcp [--library DIR | --demo] [--full] [FILES…]
      MCP server on stdio for AI agents (Claude Code: `claude mcp add br -- br mcp`).
      Without --library or --demo the photos live in memory (FILES are imported and selected).
      --full also lists every LightCraft command as a cmd_* tool (about 330 tools).
      A library open in the BR window cannot be opened here at the same time.

  br photo IN [-o OUT] [--auto] [--look LOOK[:AMOUNT]] [--light N] [--contrast N] [--color N]
           [--warmth N] [--sharpness N] [--vignette N] [--format F] [--preset P]
      Edit one photo and export it as JPEG. Sliders are -100..100. F: original, 1:1, 4:5, 9:16,
      16:9. P: instagram (1080x1350), story (1080x1920), square, wide, full (default).
      OUT is a .jpg/.jpeg file or a folder (default: $BR_EXPORTS or ~/Pictures/BR Exports).

  br run [--library DIR | --demo] [--import PATH]… [--keep-going] CMD [key=value…] [CMD …]
      Run commands and print one JSON line each: BR's br.* commands (see `br commands`) or any
      LightCraft command (develop.set, library.select, …). Values are JSON, else text
      (dir, path, look, format and preset are always text). Imported photos are selected.

  br commands         BR's commands and their parameters
  br looks [--json]   the looks
  br snapshot [--library DIR | --demo] [--popover looks|format|export]
              [-o OUT.png] [--size WxH] [--scale S] [FILES…]
      Draw the window without a display (no GPU needed) into a PNG.
";

/// Warnings and errors to stderr (stdout is reserved for results and the MCP protocol).
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn || (m.target().starts_with("br") && m.level() <= log::Level::Info)
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("[{}] {}: {}", r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

static LOGGER: StderrLog = StderrLog;

/// Print to stdout. When the reader went away (`br looks | head -2`), stop quietly.
fn out(text: impl std::fmt::Display) -> Result<(), String> {
    let mut o = std::io::stdout().lock();
    match write!(o, "{text}").and_then(|()| o.flush()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => std::process::exit(0),
        r => r.map_err(|e| e.to_string()),
    }
}

/// [`out`] with a newline.
fn outln(text: impl std::fmt::Display) -> Result<(), String> {
    out(format!("{text}\n"))
}

fn main() {
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Info));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match args.first().map(String::as_str) {
        Some("mcp" | "photo" | "run" | "commands" | "looks" | "snapshot" | "app" | "help") => (args[0].clone(), args[1..].to_vec()),
        _ => ("app".to_string(), args),
    };
    let r = match cmd.as_str() {
        "help" => out(USAGE),
        _ if rest.iter().any(|a| a == "--help" || a == "-h") => out(USAGE),
        _ if rest.iter().any(|a| a == "--version" || a == "-V") => outln(format!("br {}", env!("CARGO_PKG_VERSION"))),
        "mcp" => mcp(&rest),
        "photo" => photo(&rest),
        "run" => run(&rest),
        "commands" => commands::COMMANDS.iter().try_for_each(|(id, label, params)| outln(format!("{id:<12} {label}\n             {params}"))),
        "looks" => looks(&rest),
        "snapshot" => snapshot(&rest),
        _ => window(&rest),
    };
    if let Err(e) = r {
        eprintln!("br: {e}");
        std::process::exit(1);
    }
}

/// Options shared by the commands that open photos.
#[derive(Default)]
struct Opts {
    library: Option<PathBuf>,
    demo: bool,
    files: Vec<String>,
}

/// Take `--library DIR` / `--library=DIR` / `--demo` out of `args`; returns what is left.
fn library_opts(args: &[String], opts: &mut Opts) -> Result<Vec<String>, String> {
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let dir = match a.as_str() {
            "--demo" | "--memory" => {
                opts.demo = true;
                continue;
            }
            "--library" => it.next().map(String::as_str),
            _ if a.starts_with("--library=") => Some(&a["--library=".len()..]),
            _ => {
                rest.push(a.clone());
                continue;
            }
        };
        // an empty value (an unset variable in a script) must not make a library in the current folder
        let dir = dir.filter(|d| !d.trim().is_empty()).ok_or("--library needs a folder")?;
        opts.library = Some(PathBuf::from(dir));
    }
    if opts.demo && opts.library.is_some() {
        return Err("--demo and --library exclude each other".into());
    }
    Ok(rest)
}

/// A session as the options ask for: a library, the demo, or (`default_library`) BR's library.
/// Returns the session, where it lives, and why the library could not be opened.
fn open_session(opts: &Opts, default_library: bool) -> (Session, Option<String>, Option<(String, String)>) {
    if opts.demo {
        return (library::demo_session(), None, None);
    }
    let dir = opts.library.clone().or_else(|| if default_library { library::default_dir() } else { None });
    let Some(dir) = dir else { return (library::new_session(), None, None) };
    match library::open(&dir) {
        Ok(s) => (s, Some(dir.to_string_lossy().into_owned()), None),
        Err(e) => (library::new_session(), None, Some((dir.to_string_lossy().into_owned(), e.to_string()))),
    }
}

/// Import `paths` (files or folders) and select what they are in the library, new or already
/// there, so the next commands work on them. Fails when nothing could be imported, so commands
/// never fall through to some other photo. Returns LightCraft's import report.
fn import_and_select(s: &mut Session, paths: &[String]) -> Result<Value, String> {
    let r = s.execute("library.import", &json!({"paths": expand_paths(paths)})).map_err(|e| e.to_string())?;
    let mut ids: Vec<u64> = Vec::new();
    for v in r["imported"].as_array().into_iter().chain(r["restored"].as_array()).flatten() {
        if let Some(id) = v.as_u64().filter(|id| !ids.contains(id)) {
            ids.push(id);
        }
    }
    for d in r["duplicates"].as_array().into_iter().flatten() {
        if let Some(id) = d["existing"].as_u64().filter(|id| !ids.contains(id)) {
            ids.push(id);
        }
    }
    let Some(first) = ids.first() else {
        let why = match r["failed"].as_array().and_then(|f| f.first()) {
            Some(f) => format!("{}: {}", f[0].as_str().unwrap_or_default(), f[1].as_str().unwrap_or_default()),
            None => "no photos found".into(),
        };
        return Err(format!("could not import {}: {why}", paths.join(", ")));
    };
    s.execute("library.select", &json!({"ids": ids, "active": first})).map_err(|e| e.to_string())?;
    for f in r["failed"].as_array().into_iter().flatten() {
        log::warn!("not imported: {}: {}", f[0].as_str().unwrap_or_default(), f[1].as_str().unwrap_or_default());
    }
    Ok(r)
}

/// Window options for `renderer`. wgpu gets LightCraft's window backends (Vulkan, Metal or DX12,
/// plus OpenGL where LightCraft uses it).
fn native_options(renderer: eframe::Renderer) -> eframe::NativeOptions {
    let mut o = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(br_ui::TITLE)
            .with_app_id("br")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1000.0, 640.0])
            .with_drag_and_drop(true),
        renderer,
        ..Default::default()
    };
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(n) = &mut o.wgpu_options.wgpu_setup {
        use lightcraft_engine::gpu::backend;
        n.instance_descriptor.backends = backend::window_backends();
        n.instance_descriptor.backend_options = backend::backend_options();
        n.instance_descriptor.flags = backend::instance_flags_from_env(n.instance_descriptor.flags);
    }
    // winit has file drag-and-drop on X11 only: use XWayland where there is one (as LightCraft)
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty()) && std::env::var_os("BR_WAYLAND").is_none() {
        o.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    o
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into())
}

fn window(args: &[String]) -> Result<(), String> {
    let mut opts = Opts::default();
    let rest = library_opts(args, &mut opts)?;
    if let Some(bad) = rest.iter().find(|a| a.starts_with('-')) {
        return Err(format!("unknown option `{bad}` (see br --help)"));
    }
    opts.files = rest;
    let files = expand_paths(&opts.files);
    let forced = std::env::var("BR_RENDERER").ok().map(|r| r.to_lowercase());
    let renderer = match forced.as_deref() {
        Some("glow" | "gl" | "opengl") => eframe::Renderer::Glow,
        Some("wgpu") | None => eframe::Renderer::Wgpu,
        Some(other) => return Err(format!("BR_RENDERER={other}: use glow or wgpu")),
    };
    let (session, lib, problem) = open_session(&opts, true);
    // set once eframe has made the window and its renderer: failures after that are not the
    // renderer's start, and are never answered with a restart
    let up = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let came_up = up.clone();
    let started = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native(
            br_ui::TITLE,
            native_options(renderer),
            Box::new(move |cc| {
                came_up.store(true, std::sync::atomic::Ordering::Relaxed);
                let mut app = br_ui::BrApp::with_services(session, lib, br_ui::desktop::services(&cc.egui_ctx));
                match problem {
                    // the photos are imported once the user decides what to do about the library
                    Some((dir, e)) => app.library_problem(dir, e, files),
                    None if !files.is_empty() => {
                        if let Err(e) = lightcraft_ui_egui::import::start_paths(&mut app.lc, files) {
                            log::error!("import: {e}");
                        }
                    }
                    None => {}
                }
                Ok(Box::new(app))
            }),
        )
    }));
    let was_up = up.load(std::sync::atomic::Ordering::Relaxed);
    let failure = match started {
        Ok(Ok(())) => return Ok(()),
        // no display, or a broken X11/Wayland setup: another renderer cannot help
        Ok(Err(e @ (eframe::Error::WinitEventLoop(_) | eframe::Error::Winit(_)))) => return Err(format!("could not open the window: {e}")),
        Ok(Err(e)) if was_up => return Err(format!("BR stopped: {e}")),
        // a crash while working is a crash, not a renderer that would not start
        Err(p) if was_up => std::panic::resume_unwind(p),
        Ok(Err(e)) => e.to_string(),
        Err(p) => format!("the renderer crashed: {}", panic_text(&*p)),
    };
    if forced.is_some() || renderer != eframe::Renderer::Wgpu {
        return Err(format!("could not open the window: {failure} (try BR_RENDERER=glow or BR_RENDERER=wgpu)"));
    }
    // a failed or crashed wgpu start can leave the event loop unusable: start again with OpenGL
    log::warn!("the window could not start with wgpu ({failure}); starting again with OpenGL");
    let exe = std::env::current_exe().map_err(|e| format!("could not open the window: {failure}; and not restart: {e}"))?;
    let mut again = std::process::Command::new(exe);
    again.args(std::env::args_os().skip(1)).env("BR_RENDERER", "glow");
    // on Unix the new start takes this process's place (same pid: whoever started BR can stop it)
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = again.exec();
        Err(format!("could not open the window: {failure}; and not restart: {e}"))
    }
    #[cfg(not(unix))]
    {
        let status = again.status().map_err(|e| format!("could not open the window: {failure}; and not restart: {e}"))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn mcp(args: &[String]) -> Result<(), String> {
    let mut opts = Opts::default();
    let mut full = false;
    let mut files = Vec::new();
    for a in library_opts(args, &mut opts)? {
        match a.as_str() {
            "--full" => full = true,
            "--compact" => full = false,
            _ if a.starts_with('-') => return Err(format!("unknown option `{a}` (see br --help)")),
            _ => files.push(a),
        }
    }
    let (session, lib, problem) = open_session(&opts, false);
    if let Some((dir, e)) = problem {
        return Err(format!("{e} ({dir}). Close the BR window first, or run without --library to work in memory."));
    }
    let origin = match (&lib, opts.demo) {
        (Some(dir), _) => format!("library at {dir}"),
        (None, true) => "LightCraft's demo photos in memory".to_string(),
        (None, false) => "photos in memory (import them first; nothing is kept after the server stops)".to_string(),
    };
    let mut backend = BrBackend::new(Headless::new(session), origin);
    if !files.is_empty() {
        let r = import_and_select(&mut backend.inner.session, &files)?;
        log::info!(
            "imported {} photo(s), {} already in the library",
            r["imported"].as_array().map_or(0, Vec::len),
            r["duplicates"].as_array().map_or(0, Vec::len)
        );
    }
    br_mcp::serve(backend, full, std::io::stdin().lock(), std::io::stdout()).map_err(|e| e.to_string())
}

fn looks(args: &[String]) -> Result<(), String> {
    let s = Session::new();
    if args.iter().any(|a| a == "--json") {
        return outln(br_photo::looks::list_json(&s));
    }
    let mut group = String::new();
    for l in br_photo::looks::list(&s) {
        if l.group != group {
            outln(&l.group)?;
            group = l.group.clone();
        }
        outln(format!("  {:<24} {}", l.id, l.name))?;
    }
    Ok(())
}

fn number(v: Option<&String>, flag: &str) -> Result<f64, String> {
    v.ok_or(format!("{flag} needs a value"))?.parse::<f64>().map_err(|_| format!("{flag} needs a number"))
}

/// Where `br photo -o OUT` writes: a .jpg/.jpeg file, or a folder (one that exists, or a name
/// without an extension). Anything else is refused rather than made into a folder.
fn photo_target(out: Option<PathBuf>) -> Result<Target, String> {
    let Some(p) = out else { return Ok(Target::Dir(br_photo::export::default_dir())) };
    if p.is_dir() {
        return Ok(Target::Dir(p));
    }
    match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("jpg" | "jpeg") => Ok(Target::File(p)),
        None => Ok(Target::Dir(p)),
        Some(e) => Err(format!("br photo writes JPEG: `{}` must end in .jpg/.jpeg or be a folder (not .{e})", p.display())),
    }
}

fn photo(args: &[String]) -> Result<(), String> {
    let mut input = None;
    let mut out = None;
    let mut auto = false;
    let mut look: Option<(String, f64)> = None;
    let mut sliders: Vec<(Slider, f64)> = Vec::new();
    let mut format = None;
    let mut preset = Preset::Full;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out = Some(PathBuf::from(it.next().ok_or("-o needs a file or folder")?)),
            "--auto" => auto = true,
            "--look" => {
                let v = it.next().ok_or("--look needs a look (see br looks)")?;
                look = Some(match v.rsplit_once(':').and_then(|(l, a)| Some((l, a.parse::<f64>().ok()?))) {
                    Some((l, amount)) => (l.to_string(), amount),
                    None => (v.clone(), 100.0),
                });
            }
            "--format" => {
                let v = it.next().ok_or("--format needs a format")?;
                format = Some(Format::parse(v).ok_or(format!("unknown format `{v}`: original, 1:1, 4:5, 9:16 or 16:9"))?);
            }
            "--preset" => {
                let v = it.next().ok_or("--preset needs a preset")?;
                preset = Preset::parse(v).ok_or(format!("unknown preset `{v}`: instagram, story, square, wide or full"))?;
            }
            _ if a.starts_with("--") && Slider::parse(&a[2..]).is_some() => {
                let k = Slider::parse(&a[2..]).unwrap_or(Slider::Light);
                let v = number(it.next(), a)?;
                if !(-100.0..=100.0).contains(&v) {
                    return Err(format!("{a} must be from -100 to 100"));
                }
                sliders.push((k, v));
            }
            _ if a.starts_with('-') => return Err(format!("unknown option `{a}` (see br --help)")),
            _ if input.is_none() => input = Some(a.clone()),
            _ => return Err(format!("one photo at a time (got `{a}` too)")),
        }
    }
    let input = input.ok_or("which photo? br photo IN [-o OUT] …")?;
    if !Path::new(&input).is_file() {
        return Err(format!("no such file: {input}"));
    }
    let target = photo_target(out)?;
    let mut s = library::new_session();
    let r = import_and_select(&mut s, std::slice::from_ref(&input))?;
    let id = s
        .active()
        .ok_or_else(|| format!("could not open {input}: {}", r["failed"].get(0).map(|f| f[1].to_string()).unwrap_or_else(|| "not a photo".into())))?;
    let mut st = br_photo::BrState::default();
    if auto {
        br_photo::adjust::auto(&mut s, id).map_err(|e| e.to_string())?;
    }
    if let Some((l, amount)) = &look {
        br_photo::looks::set_look(&mut s, &mut st.look, id, l, *amount, false).map_err(|e| e.to_string())?;
    }
    if !sliders.is_empty() {
        br_photo::adjust::set(&mut s, id, &sliders).map_err(|e| e.to_string())?;
    }
    if let Some(f) = format {
        br_photo::crop::apply(&mut s, id, f).map_err(|e| e.to_string())?;
    }
    let files = br_photo::export::export(&mut s, id, preset, &target)?;
    for f in &files {
        outln(json!({"path": f["path"], "width": f["width"], "height": f["height"], "preset": preset.key()}))?;
    }
    Ok(())
}

/// One command of `br run`.
struct Step {
    id: String,
    params: Value,
}

/// Params that are always text, so `dir=2024` is the folder "2024", not a number.
const TEXT_PARAMS: &[&str] = &["dir", "path", "look", "format", "preset"];

fn parse_steps(tokens: &[String]) -> Result<Vec<Step>, String> {
    let mut steps: Vec<Step> = Vec::new();
    for t in tokens {
        if t.starts_with('{') {
            let v: Value = serde_json::from_str(t).map_err(|e| format!("bad JSON params `{t}`: {e}"))?;
            let step = steps.last_mut().ok_or_else(|| format!("params `{t}` before any command"))?;
            match (step.params.as_object_mut(), v) {
                (Some(o), Value::Object(m)) => o.extend(m),
                _ => return Err(format!("params `{t}` must be a JSON object")),
            }
        } else if let Some((k, v)) = t.split_once('=') {
            let step = steps.last_mut().ok_or_else(|| format!("param `{t}` before any command"))?;
            let k = k.trim();
            step.params[k] = if TEXT_PARAMS.contains(&k) { json!(v) } else { serde_json::from_str(v).unwrap_or_else(|_| json!(v)) };
        } else {
            steps.push(Step { id: t.clone(), params: json!({}) });
        }
    }
    Ok(steps)
}

fn run(args: &[String]) -> Result<(), String> {
    let mut opts = Opts::default();
    let mut imports = Vec::new();
    let mut keep_going = false;
    let mut tokens = Vec::new();
    let rest = library_opts(args, &mut opts)?;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--import" => imports.push(it.next().ok_or("--import needs a path")?.clone()),
            "--keep-going" => keep_going = true,
            _ if a.starts_with("--") && tokens.is_empty() => return Err(format!("unknown option `{a}` (see br --help)")),
            _ => tokens.push(a.clone()),
        }
    }
    let steps = parse_steps(&tokens)?;
    if steps.is_empty() {
        return Err("no command: br run [--demo] CMD key=value … (see br commands)".into());
    }
    let (session, _, problem) = open_session(&opts, false);
    if let Some((dir, e)) = problem {
        return Err(format!("{e} ({dir})"));
    }
    let mut b = BrBackend::new(Headless::new(session), "br run");
    if !imports.is_empty() {
        import_and_select(&mut b.inner.session, &imports)?;
    }
    let mut failed = false;
    for step in steps {
        let t = std::time::Instant::now();
        let r = b.call("engine.execute", json!({"command": step.id, "params": step.params}));
        let ms = t.elapsed().as_millis();
        match r {
            Ok(v) => outln(json!({"command": step.id, "ok": true, "result": v, "ms": ms}))?,
            Err(e) => {
                failed = true;
                outln(json!({"command": step.id, "ok": false, "error": e, "ms": ms}))?;
                if !keep_going {
                    break;
                }
            }
        }
    }
    if failed { Err("a command failed".into()) } else { Ok(()) }
}

fn snapshot(args: &[String]) -> Result<(), String> {
    let mut opts = Opts::default();
    let mut out = PathBuf::from("br-snapshot.png");
    let mut size = [1440.0f32, 900.0];
    let mut scale = 1.0f32;
    let mut popover = None;
    let rest = library_opts(args, &mut opts)?;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out = PathBuf::from(it.next().ok_or("-o needs a file")?),
            "--popover" => {
                let v = it.next().ok_or("--popover needs a tool")?;
                popover = Some(br_ui::Popover::ALL.into_iter().find(|p| p.key() == v).ok_or(format!("unknown tool `{v}`: looks, format or export"))?);
            }
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                size = [w.parse().map_err(|_| "--size needs WxH")?, h.parse().map_err(|_| "--size needs WxH")?];
            }
            "--scale" => scale = number(it.next(), "--scale")? as f32,
            _ if a.starts_with('-') => return Err(format!("unknown option `{a}` (see br --help)")),
            _ => opts.files.push(a.clone()),
        }
    }
    lightcraft_ui_egui::headless::viewport_pixels(size, scale).map_err(|e| format!("--size/--scale: {e}"))?;
    if opts.library.is_none() && opts.files.is_empty() {
        opts.demo = true;
    }
    lightcraft_engine::gpu::set_enabled(false);
    let (session, lib, problem) = open_session(&opts, false);
    if let Some((dir, e)) = problem {
        return Err(format!("{e} ({dir})"));
    }
    let mut app = br_ui::BrApp::new(session, lib);
    app.lc.ui.settings.gpu = false;
    if !opts.files.is_empty() {
        import_and_select(&mut app.lc.session, &opts.files)?;
    }
    let mut d = br_ui::headless::Driver::new(size, scale);
    d.settle(&mut app, Duration::from_secs(120));
    if let Some(p) = popover {
        // open it the way a click does: under its button in the header
        let button = if p == br_ui::Popover::Export { "export".to_string() } else { format!("tool.{}", p.key()) };
        d.click(&mut app, &button)?;
        d.settle(&mut app, Duration::from_secs(120));
    }
    let png = br_ui::headless::png(&d.paint());
    std::fs::write(&out, png).map_err(|e| format!("{}: {e}", out.display()))?;
    app.on_exit();
    outln(out.display())
}
