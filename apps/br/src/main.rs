//! `br`: BR's window and command line.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Duration;

use br_mcp::{BrBackend, BrServer};
use br_photo::adjust::Slider;
use br_photo::crop::Format;
use br_photo::export::{Preset, Target};
use br_photo::{commands, library};
use lightcraft_engine::Session;
use lightcraft_mcp::{Backend, Headless, expand_paths};
use serde_json::{Value, json};

const USAGE: &str = "\
BR — eine Media-Suite. BR Foto: einfache Fotobearbeitung, mit LightCraft als Pro-Modus.

USAGE
  br [--library DIR | --demo] [FILES…]
      Open the window. FILES (photos or folders) are imported. The library is DIR, else
      $BR_LIBRARY, else ~/Pictures/BR Library; --demo opens LightCraft's demo photos in memory.

  br mcp [--library DIR | --demo] [--full] [FILES…]
      MCP server on stdio for AI agents (Claude Code: `claude mcp add br -- br mcp`).
      Without --library or --demo the photos live in memory (FILES are imported).
      --full also lists every LightCraft command as a cmd_* tool (about 330 tools).
      A library open in the BR window cannot be opened here at the same time.

  br photo IN [-o OUT] [--auto] [--look LOOK[:AMOUNT]] [--light N] [--contrast N] [--color N]
           [--warmth N] [--sharpness N] [--vignette N] [--format F] [--preset P]
      Edit one photo and export it as JPEG. Sliders are -100..100. F: original, 1:1, 4:5, 9:16,
      16:9. P: instagram (1080x1350), story (1080x1920), square, wide, full (default).
      OUT is a .jpg file or a folder (default: $BR_EXPORTS or ~/Pictures/BR Exports).

  br run [--library DIR | --demo] [--import PATH]… CMD [key=value…] [CMD …]
      Run commands and print one JSON line each: BR's br.* commands (see `br commands`) or any
      LightCraft command (develop.set, library.select, …). Values are JSON, else text.

  br commands         BR's commands and their parameters
  br looks [--json]   the looks
  br snapshot [--library DIR | --demo] [--pro] [--tab looks|adjust|format|export]
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

fn main() {
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Info));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match args.first().map(String::as_str) {
        Some("mcp" | "photo" | "run" | "commands" | "looks" | "snapshot" | "app" | "help") => (args[0].clone(), args[1..].to_vec()),
        _ => ("app".to_string(), args),
    };
    let r = match cmd.as_str() {
        "help" => {
            print!("{USAGE}");
            Ok(())
        }
        _ if rest.iter().any(|a| a == "--help" || a == "-h") => {
            print!("{USAGE}");
            Ok(())
        }
        _ if rest.iter().any(|a| a == "--version" || a == "-V") => {
            println!("br {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "mcp" => mcp(&rest),
        "photo" => photo(&rest),
        "run" => run(&rest),
        "commands" => {
            for (id, label, params) in commands::COMMANDS {
                println!("{id:<12} {label}\n             {params}");
            }
            Ok(())
        }
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
        match a.as_str() {
            "--demo" | "--memory" => opts.demo = true,
            "--library" => opts.library = Some(PathBuf::from(it.next().ok_or("--library needs a folder")?)),
            _ if a.starts_with("--library=") => opts.library = Some(PathBuf::from(&a["--library=".len()..])),
            _ => rest.push(a.clone()),
        }
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

/// Window options for `renderer`. wgpu gets LightCraft's window backends (Vulkan, Metal or DX12,
/// plus OpenGL), so machines without Vulkan still get a window.
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
    o
}

/// The renderers to try, in order: `BR_RENDERER=glow` or `=wgpu` picks one.
fn renderers() -> Vec<eframe::Renderer> {
    match std::env::var("BR_RENDERER").ok().as_deref().map(str::to_lowercase).as_deref() {
        Some("glow" | "gl" | "opengl") => vec![eframe::Renderer::Glow],
        Some("wgpu") => vec![eframe::Renderer::Wgpu],
        _ => vec![eframe::Renderer::Wgpu, eframe::Renderer::Glow],
    }
}

fn window(args: &[String]) -> Result<(), String> {
    let mut opts = Opts::default();
    let rest = library_opts(args, &mut opts)?;
    if let Some(bad) = rest.iter().find(|a| a.starts_with('-')) {
        return Err(format!("unknown option `{bad}` (see br --help)"));
    }
    opts.files = rest;
    let files = expand_paths(&opts.files);
    let renderers = renderers();
    let mut last = String::new();
    for (i, renderer) in renderers.iter().enumerate() {
        // a fresh session per attempt: a failed start drops the last one (and its library lock)
        let (session, lib, problem) = open_session(&opts, true);
        let files = files.clone();
        let r = eframe::run_native(
            br_ui::TITLE,
            native_options(*renderer),
            Box::new(move |_cc| {
                let mut app = br_ui::BrApp::new(session, lib);
                if let Some((dir, e)) = problem {
                    app.library_problem(dir, e);
                }
                if !files.is_empty()
                    && let Err(e) = lightcraft_ui_egui::import::start_paths(&mut app.lc, files)
                {
                    log::error!("import: {e}");
                }
                Ok(Box::new(app))
            }),
        );
        match r {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = e.to_string();
                if i + 1 < renderers.len() {
                    log::warn!("the window could not start with {renderer}: {e}; trying the next renderer");
                }
            }
        }
    }
    Err(format!("could not open the window: {last} (try BR_RENDERER=glow or BR_RENDERER=wgpu)"))
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
        let r = backend.inner.session.execute("library.import", &json!({"paths": expand_paths(&files)})).map_err(|e| e.to_string())?;
        log::info!("imported {} photo(s)", r["imported"].as_array().map_or(0, Vec::len));
    }
    let mut server = BrServer::new(backend, full);
    server.serve(BufReader::new(std::io::stdin()), std::io::stdout()).map_err(|e| e.to_string())
}

fn looks(args: &[String]) -> Result<(), String> {
    let s = Session::new();
    if args.iter().any(|a| a == "--json") {
        println!("{}", br_photo::looks::list_json(&s));
    } else {
        let mut group = String::new();
        for l in br_photo::looks::list(&s) {
            if l.group != group {
                println!("{}", l.group);
                group = l.group.clone();
            }
            println!("  {:<24} {}", l.id, l.name);
        }
    }
    Ok(())
}

fn number(v: Option<&String>, flag: &str) -> Result<f64, String> {
    v.ok_or(format!("{flag} needs a value"))?.parse::<f64>().map_err(|_| format!("{flag} needs a number"))
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
                look = Some(match v.rsplit_once(':') {
                    Some((l, amount)) if amount.parse::<f64>().is_ok() => (l.to_string(), amount.parse().unwrap_or(100.0)),
                    _ => (v.clone(), 100.0),
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
    let mut s = library::new_session();
    let r = s.execute("library.import", &json!({"paths": expand_paths(std::slice::from_ref(&input))})).map_err(|e| e.to_string())?;
    let id =
        r["imported"].get(0).and_then(Value::as_u64).map(lightcraft_engine::catalog::PhotoId).ok_or_else(|| {
            format!("could not open {input}: {}", r["failed"].get(0).map(|f| f[1].to_string()).unwrap_or_else(|| "not a photo".into()))
        })?;
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
    let target = match out {
        Some(p) if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")) => Target::File(p),
        Some(p) => Target::Dir(p),
        None => Target::Dir(br_photo::export::default_dir()),
    };
    let files = br_photo::export::export(&mut s, id, preset, &target)?;
    for f in &files {
        println!("{}", json!({"path": f["path"], "width": f["width"], "height": f["height"], "preset": preset.key()}));
    }
    Ok(())
}

/// One command of `br run`.
struct Step {
    id: String,
    params: Value,
}

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
            step.params[k.trim()] = serde_json::from_str(v).unwrap_or_else(|_| json!(v));
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
        b.inner.session.execute("library.import", &json!({"paths": expand_paths(&imports)})).map_err(|e| e.to_string())?;
    }
    let mut failed = false;
    for step in steps {
        let t = std::time::Instant::now();
        let r = b.call("engine.execute", json!({"command": step.id, "params": step.params}));
        let ms = t.elapsed().as_millis();
        match r {
            Ok(v) => println!("{}", json!({"command": step.id, "ok": true, "result": v, "ms": ms})),
            Err(e) => {
                failed = true;
                println!("{}", json!({"command": step.id, "ok": false, "error": e, "ms": ms}));
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
    let mut pro = false;
    let mut tab = None;
    let rest = library_opts(args, &mut opts)?;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out = PathBuf::from(it.next().ok_or("-o needs a file")?),
            "--pro" => pro = true,
            "--tab" => {
                let v = it.next().ok_or("--tab needs a tab")?;
                tab = Some(br_ui::Tab::ALL.into_iter().find(|t| t.key() == v).ok_or(format!("unknown tab `{v}`: looks, adjust, format or export"))?);
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
    if opts.library.is_none() && opts.files.is_empty() {
        opts.demo = true;
    }
    lightcraft_engine::gpu::set_enabled(false);
    let (session, lib, problem) = open_session(&opts, false);
    let mut app = br_ui::BrApp::new(session, lib);
    app.lc.ui.settings.gpu = false;
    if let Some((dir, e)) = problem {
        return Err(format!("{e} ({dir})"));
    }
    if !opts.files.is_empty() {
        app.lc.session.execute("library.import", &json!({"paths": expand_paths(&opts.files)})).map_err(|e| e.to_string())?;
    }
    if let Some(tab) = tab {
        app.simple.tab = tab;
    }
    let mut d = br_ui::headless::Driver::new(size, scale);
    d.settle(&mut app, Duration::from_secs(120));
    if pro {
        app.open_pro();
        d.settle(&mut app, Duration::from_secs(120));
    }
    let png = br_ui::headless::png(&d.paint());
    std::fs::write(&out, png).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("{}", out.display());
    app.on_exit();
    Ok(())
}
