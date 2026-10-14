//! `br`: BR's window and command line.

mod logging;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use br_mcp::BrBackend;
use br_photo::adjust::Slider;
use br_photo::crop::Format;
use br_photo::export::{Preset, Target};
use br_photo::{commands, library};
use br_ui::desktop::prefs;
use lightcraft_engine::Session;
use lightcraft_mcp::{Backend, Headless, expand_paths};
use lightcraft_ui_egui::UiState;
use serde_json::{Value, json};

const USAGE: &str = "\
BR — eine Media-Suite: DaVinci-Umfang, so direkt bedienbar wie CapCut. Seiten: Medien, Schnitt,
Farbe, Ton, Ausgabe (Video, FilmCraft) und Foto (LightCraft).

USAGE
  br [--library DIR | --demo] [--control [PORT]] [FILES…]
      Open the window on the Schnitt page. FILES: videos and sound are added to the video (a new
      one in the first clip's shape), a video project (.fcproj, EDL, XML, FCPXML, OTIO) is opened,
      subtitle files (SRT, VTT, SCC) become captions; photos and folders are imported into the
      photo library (folders: their photos), and with photos alone the window opens on the Foto
      page. The library is DIR, else
      $BR_LIBRARY, else the library last opened in the window, else ~/Pictures/BR Library;
      --demo opens LightCraft's demo photos in memory and saves nothing.
      BR_RENDERER=gl draws the window with OpenGL (Linux and Windows: drivers without Vulkan, software rendering),
      =wgpu with the GPU's own API; BR_WAYLAND=1 keeps native Wayland (by default BR uses X11
      where it can, which is what makes dropping files work).
      BR_IMPORT_DIR is the folder the import dialog opens in (see deploy/browser for a BR
      on a server, used in the browser).
      The window keeps LightCraft's settings in BR's settings folder (Linux: ~/.config/BR) and
      its log in BR's log folder (Linux: ~/.local/share/BR/Logs; Help ▸ Open Log Folder).
      BR_CONFIG_DIR=DIR keeps both in DIR; set empty, nothing is kept.
      --control (or BR_CONTROL=1, or a port) opens the window's control channel on 127.0.0.1
      for `br mcp --connect`: AI agents then work in this window. It is guarded by a token made
      at each start and kept, readable only by you, in the settings folder (control.json).

  br mcp [--library DIR | --demo | --connect [HOST:PORT]] [--full] [FILES…]
      MCP server on stdio for AI agents (Claude Code: `claude mcp add br -- br mcp`): photos
      (br_* tools, LightCraft's) and video (br_video_* tools, film_commands/film_run for all of
      FilmCraft). Without --library or --demo the photos live in memory; FILES: photos are
      imported and selected, videos, sound and subtitles go into the video.
      --connect works in a running window instead (one started with --control): every tool
      edits what the user sees. Without HOST:PORT it reads the window's port and token from
      BR's settings folder (control.json); with it the token comes from $BR_CONTROL_TOKEN.
      --full also lists every LightCraft command as a cmd_* tool (about 330 tools).
      A library open in the BR window cannot be opened here at the same time.

  br photo IN [-o OUT] [--auto] [--look LOOK[:AMOUNT]] [--light N] [--contrast N] [--color N]
           [--warmth N] [--sharpness N] [--vignette N] [--format F] [--preset P]
      Edit one photo and export it as JPEG. Sliders are -100..100. F: original, 1:1, 4:5, 9:16,
      16:9. P: instagram (1080x1350), story (1080x1920), square, wide, full (default).
      OUT is a .jpg/.jpeg file or a folder (default: $BR_EXPORTS or ~/Pictures/BR Exports).

  br run [--library DIR | --demo] [--import PATH]… [--keep-going] CMD [key=value…] [CMD …]
      Run commands and print one JSON line each: BR's br.* photo commands and br.video.* video
      commands (see `br commands`), any LightCraft command (develop.set, library.select, …) and
      any FilmCraft command as film.<id> (film.timeline.razor, …). Values are JSON, else text
      (dir, path, look, format, preset, text, language, model, name and fit are always text).
      Imported photos are selected;
      imported videos, sound and subtitles go into the video (as br.video.add).

  br commands [--film]  BR's commands and their parameters (--film: FilmCraft's)
  br looks [--json]   the looks
  br snapshot [--library DIR | --demo] [--page PAGE] [--popover looks|format|export]
              [-o OUT.png] [--size WxH] [--scale S] [FILES…]
      Draw the window without a display (no GPU needed) into a PNG. PAGE: medien, schnitt
      (default), farbe, ton, ausgabe, foto (default with photo FILES and no videos, and with
      --popover: the Foto tools).

  Logging goes to standard error (and the window's log file), never to standard output:
  warnings, and information from BR and LightCraft. BR_LOG=debug|info|warn sets the level of
  BR's and LightCraft's own records, else RUST_LOG takes env_logger directives.
";

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

/// Give memory the system allocator keeps after frees back to the system, which LightCraft asks
/// for after big decodes (`lightcraft_engine::memory::set_release_hook`, as LightCraft's apps
/// install it): macOS keeps freed large blocks resident until it runs short.
fn release_free_memory() {
    let _ = lightcraft_sysmem::release_free_memory();
}

fn main() {
    // first, so every start-up record is kept for the window's log file (`logging`)
    let logger = logging::install();
    lightcraft_engine::memory::set_release_hook(release_free_memory);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match args.first().map(String::as_str) {
        Some("mcp" | "photo" | "run" | "commands" | "looks" | "snapshot" | "app" | "help") => (args[0].clone(), args[1..].to_vec()),
        _ => ("app".to_string(), args),
    };
    // only the window keeps a log file
    if let Some(logger) = logger.filter(|_| cmd != "app") {
        logger.no_file();
    }
    let r = match cmd.as_str() {
        "help" => out(USAGE),
        _ if rest.iter().any(|a| a == "--help" || a == "-h") => out(USAGE),
        _ if rest.iter().any(|a| a == "--version" || a == "-V") => outln(format!("br {}", env!("CARGO_PKG_VERSION"))),
        "mcp" => mcp(&rest),
        "photo" => photo(&rest),
        "run" => run(&rest),
        "commands" => list_commands(&rest),
        "looks" => looks(&rest),
        "snapshot" => snapshot(&rest),
        _ => window(&rest, logger),
    };
    if let Err(e) = r {
        if let Some(logger) = logger.filter(|_| cmd == "app") {
            logger.record_error(&e);
        }
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

/// How the window draws: wgpu on the system's GPU API (Vulkan, Metal or DX12, plus OpenGL where
/// LightCraft uses it), or wgpu on OpenGL only (`BR_RENDERER=gl`: drivers without Vulkan, Mesa's
/// software rendering on a server). There is no separate OpenGL renderer: FilmCraft is built
/// for eframe's wgpu renderer alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Renderer {
    Wgpu,
    Gl,
}

/// Window options for `renderer`.
fn native_options(renderer: Renderer) -> eframe::NativeOptions {
    let mut o = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(br_ui::TITLE)
            .with_app_id("br")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1000.0, 640.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(n) = &mut o.wgpu_options.wgpu_setup {
        use lightcraft_engine::gpu::backend;
        n.instance_descriptor.backends = match renderer {
            Renderer::Wgpu => backend::window_backends(),
            Renderer::Gl => eframe::wgpu::Backends::GL,
        };
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

/// The window: BR, and the writer that keeps LightCraft's settings for the next start
/// (`br_ui::desktop::prefs`): as soon as the library changes, every few seconds when anything
/// else changed, and when the window closes.
struct Window(br_ui::BrApp, prefs::PrefsWriter);

impl eframe::App for Window {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        eframe::App::logic(&mut self.0, ctx, frame);
        self.1.tick(&mut self.0.lc, ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        eframe::App::raw_input_hook(&mut self.0, ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.0, ui, frame);
    }

    fn on_exit(&mut self) {
        // first: BR's own exit ends the process on macOS
        if let Err(e) = self.1.save(&self.0.lc) {
            log::error!("{e}");
        }
        eframe::App::on_exit(&mut self.0);
    }
}

/// Start the window's log file in BR's log folder (the previous ones move up to `br.1.log` …);
/// its path, or `None` when this run keeps none: `--demo` writes nothing, and `BR_CONFIG_DIR`
/// set empty (or no home folder) means there is no log folder.
fn open_log(logger: &logging::AppLogger, demo: bool) -> Option<PathBuf> {
    match br_ui::desktop::log_dir().filter(|_| !demo) {
        Some(dir) => match logger.attach_dir(&dir) {
            Ok(path) => {
                log::info!("BR {}, log file {}", env!("CARGO_PKG_VERSION"), path.display());
                Some(path)
            }
            // the file sink has given up by now, so this goes to standard error only
            Err(e) => {
                log::warn!("no log file: {e}");
                None
            }
        },
        None => {
            logger.no_file();
            None
        }
    }
}

/// Where LightCraft's panic guard writes its report: `br-panics.log` beside the log file, or in
/// `temp` when this run keeps no log (nothing of the user's is written then).
fn panic_log_path(log_file: Option<&Path>, temp: PathBuf) -> PathBuf {
    log_file.and_then(Path::parent).map_or(temp, Path::to_path_buf).join("br-panics.log")
}

/// The library the window opens: `--library`, else the one last opened in the window (LightCraft's
/// Settings ▸ Open Library…, kept in its settings) unless `$BR_LIBRARY` names one, else `None`:
/// BR's default library ([`open_session`]).
fn launch_library(cli: Option<PathBuf>, saved: Option<&UiState>, br_library_set: bool) -> Option<PathBuf> {
    cli.or_else(|| saved.map(|u| u.settings.library_path.as_str()).filter(|p| !p.trim().is_empty() && !br_library_set).map(PathBuf::from))
}

/// `--control [PORT]`, else `$BR_CONTROL` (a port, or 1/on for a free one): the port of the
/// window's control channel (0: a free one), `None` without one.
fn control_port(rest: &mut Vec<String>) -> Result<Option<u16>, String> {
    let parse = |v: &str| v.parse::<u16>().map_err(|_| format!("--control: `{v}` is not a port"));
    if let Some(i) = rest.iter().position(|a| a == "--control" || a.starts_with("--control=")) {
        let a = rest.remove(i);
        if let Some(v) = a.strip_prefix("--control=") {
            return parse(v).map(Some);
        }
        if rest.get(i).is_some_and(|v| v.parse::<u16>().is_ok()) {
            return parse(&rest.remove(i)).map(Some);
        }
        return Ok(Some(0));
    }
    match std::env::var("BR_CONTROL").ok().as_deref().map(str::trim) {
        None | Some("" | "0" | "off" | "no") => Ok(None),
        Some("1" | "on" | "yes") => Ok(Some(0)),
        Some(v) => parse(v).map(Some),
    }
}

/// Open the window's control channel and say where (`control.json` next to the settings, for
/// `br mcp --connect`).
fn open_control(app: br_ui::BrApp, port: u16, ctx: &egui::Context) -> br_ui::BrApp {
    let token = br_ui::control::new_token();
    let (port, channels) = match br_ui::control::start(port, token.clone(), ctx.clone()) {
        Ok(c) => c,
        Err(e) => {
            log::error!("control channel on 127.0.0.1:{port}: {e}");
            return app;
        }
    };
    match br_ui::desktop::config_dir() {
        Some(dir) => {
            let path = br_ui::control::info_path(&dir);
            match br_ui::control::write_info(&path, port, &token) {
                Ok(()) => log::info!("control channel: 127.0.0.1:{port}, `br mcp --connect` signs in with {}", path.display()),
                Err(e) => log::error!("control channel: {}: {e}", path.display()),
            }
        }
        // no settings folder (BR_CONFIG_DIR set empty): the token goes to this terminal only
        None => eprintln!("br: control channel on 127.0.0.1:{port}, token {token} (br mcp --connect 127.0.0.1:{port} with BR_CONTROL_TOKEN)"),
    }
    app.with_control(channels)
}

fn window(args: &[String], logger: Option<&'static logging::AppLogger>) -> Result<(), String> {
    let mut opts = Opts::default();
    let mut rest = library_opts(args, &mut opts)?;
    let control = control_port(&mut rest)?;
    if let Some(bad) = rest.iter().find(|a| a.starts_with('-')) {
        return Err(format!("unknown option `{bad}` (see br --help)"));
    }
    opts.files = rest;
    // videos, sound, video projects and subtitles go to the video pages; the rest to LightCraft
    let (videos, photos): (Vec<String>, Vec<String>) = opts.files.iter().cloned().partition(|f| br_video::is_video_file(f));
    let files = expand_paths(&photos);
    let forced = std::env::var("BR_RENDERER").ok().map(|r| r.to_lowercase());
    let renderer = match forced.as_deref() {
        // `glow` was the separate OpenGL renderer of earlier versions: it now means OpenGL too
        Some("gl" | "glow" | "opengl") if cfg!(target_os = "macos") => {
            return Err("BR_RENDERER=gl: OpenGL is not available on macOS (BR draws with Metal there)".into());
        }
        Some("gl" | "glow" | "opengl") => Renderer::Gl,
        Some("wgpu") | None => Renderer::Wgpu,
        Some(other) => return Err(format!("BR_RENDERER={other}: use gl or wgpu")),
    };
    // the log file, once the arguments are fine: a usage error leaves none behind
    let log_file = logger.and_then(|l| open_log(l, opts.demo));
    // a panic leaves a trace: on standard error and in br-panics.log (LightCraft's engine guard),
    // and in the log file
    lightcraft_engine::guard::install_hook(panic_log_path(log_file.as_deref(), std::env::temp_dir()));
    if let Some(logger) = logger {
        logging::record_panics(logger);
    }
    // the video side: hardware decoders, GPU exports (not after a fallback from wgpu: the driver
    // that failed is not loaded again), FilmCraft's crash log (chained after the hook above)
    br_ui::desktop::film::register_process(log_file.as_deref().and_then(|f| f.parent()).map(Path::to_path_buf), renderer == Renderer::Wgpu);
    // LightCraft's settings from the last run, read with --demo too (so it looks like the user's
    // BR) but never written then
    let demo = opts.demo;
    let prefs_path = prefs::path();
    let (saved, prefs_warning, keep_prefs_file) = match &prefs_path {
        Some(p) => prefs::load_prefs_at(p, !demo),
        None => (None, None, false),
    };
    if !demo {
        let br_library_set = std::env::var_os("BR_LIBRARY").is_some_and(|d| !d.is_empty());
        opts.library = launch_library(opts.library.take(), saved.as_ref(), br_library_set);
    }
    // GPU rendering is off from the start when the settings say so: nothing creates the device
    lightcraft_engine::gpu::set_enabled(saved.as_ref().is_none_or(|u| u.settings.gpu));
    let (mut session, lib, problem) = open_session(&opts, true);
    // the window works on its UI thread: Add to Apple Photos runs on a worker (as in LightCraft)
    session.apple_photos_background = true;
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
                // FilmCraft's auto-save first: its interface then offers to recover a crashed session
                let video = br_ui::desktop::film::session(if demo { None } else { br_ui::desktop::video_dir() });
                let mut app = br_ui::BrApp::with_sessions(session, video, lib, br_ui::desktop::services(cc, log_file.as_deref()));
                if let Some(port) = control {
                    app = open_control(app, port, &cc.egui_ctx);
                }
                // FilmCraft in the window: dialogs, the GPU, sound
                br_ui::desktop::film::attach(&mut app.fc, cc);
                // Settings ▸ General ▸ At Startup (when no video files are given)
                if videos.is_empty() && !demo {
                    br_ui::desktop::film::at_startup(&mut app.fc);
                }
                // the settings from the last run replace BR's defaults (German, for one)
                if let Some(ui) = saved {
                    app.lc.ui = ui;
                }
                app.lc.notices.extend(prefs_warning);
                // the window says when there is no GPU to render with (headless runs don't)
                app.lc.gpu_notice = true;
                // what is on disk now: only changes are written, and nothing at all with --demo
                let writer = prefs::PrefsWriter::new(prefs_path.filter(|_| !demo), &app.lc.ui, keep_prefs_file);
                let videos_given = videos.len();
                if videos_given > 0 {
                    br_ui::cut::open_files(&mut app, &cc.egui_ctx, videos);
                }
                let photos_only = videos_given == 0 && !files.is_empty();
                match problem {
                    // the photos are imported once the user decides what to do about the library
                    Some((dir, e)) => {
                        if videos_given == 0 {
                            app.open(br_ui::Page::Foto);
                        }
                        app.library_problem(dir, e, files);
                    }
                    None if !files.is_empty() => {
                        if photos_only {
                            app.open(br_ui::Page::Foto);
                        }
                        if let Err(e) = lightcraft_ui_egui::import::start_paths(&mut app.lc, files) {
                            log::error!("import: {e}");
                        }
                    }
                    None => {}
                }
                Ok(Box::new(Window(app, writer)))
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
    if forced.is_some() || renderer != Renderer::Wgpu {
        return Err(format!("could not open the window: {failure} (try BR_RENDERER=gl or BR_RENDERER=wgpu)"));
    }
    // wgpu has no OpenGL on macOS (only Metal): nothing to fall back to
    if cfg!(target_os = "macos") {
        return Err(format!("could not open the window: {failure}"));
    }
    // a failed or crashed wgpu start can leave the event loop unusable: start again with OpenGL
    log::warn!("the window could not start with wgpu ({failure}); starting again with OpenGL");
    let exe = std::env::current_exe().map_err(|e| format!("could not open the window: {failure}; and not restart: {e}"))?;
    let mut again = std::process::Command::new(exe);
    again.args(std::env::args_os().skip(1)).env("BR_RENDERER", "gl");
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
    let mut connect: Option<Option<String>> = None;
    let rest = library_opts(args, &mut opts)?;
    let mut it = rest.into_iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--full" => full = true,
            "--compact" => full = false,
            "--connect" => connect = Some(it.next_if(|v| v.contains(':') && !v.starts_with('-'))),
            _ if a.starts_with("--connect=") => connect = Some(Some(a["--connect=".len()..].to_string())),
            _ if a.starts_with('-') => return Err(format!("unknown option `{a}` (see br --help)")),
            _ => files.push(a),
        }
    }
    if let Some(addr) = connect {
        if !files.is_empty() || opts.library.is_some() || opts.demo {
            return Err("--connect works on the window's library and video: import with the import and br_video_add tools".into());
        }
        let info = br_ui::desktop::config_dir().map(|d| br_ui::control::info_path(&d));
        let window = match addr {
            None => br_mcp::Window::from_info(
                info.as_deref().ok_or("no settings folder (BR_CONFIG_DIR is empty): name the window with --connect HOST:PORT")?,
            )?,
            Some(addr) => {
                let token = match std::env::var("BR_CONTROL_TOKEN").ok().filter(|t| !t.is_empty()) {
                    Some(t) => t,
                    None => {
                        let path = info.ok_or("BR_CONTROL_TOKEN is not set and there is no settings folder")?;
                        let text = std::fs::read_to_string(&path).map_err(|e| format!("BR_CONTROL_TOKEN is not set and {}: {e}", path.display()))?;
                        serde_json::from_str::<Value>(&text).ok().and_then(|v| v["token"].as_str().map(str::to_string)).ok_or("no token")?
                    }
                };
                br_mcp::Window::new(addr, token)
            }
        };
        log::info!("br mcp: working in {}", lightcraft_mcp::Backend::describe(&window));
        return br_mcp::serve(window, full, std::io::stdin().lock(), std::io::stdout()).map_err(|e| e.to_string());
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
    // videos, sound and subtitles into the video project, the rest into the photo library
    let (videos, files): (Vec<String>, Vec<String>) = files.into_iter().partition(|f| br_video::is_video_file(f));
    if !videos.is_empty() {
        let st = br_video::open_files(&mut backend.video, &videos)?;
        log::info!("video: {st}");
    }
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
const TEXT_PARAMS: &[&str] = &["dir", "path", "look", "format", "preset", "text", "language", "model", "name", "fit"];

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
            // text stays text for these, except a list or an object (film.masks.add path=[[…]])
            let structured = v.trim_start().starts_with(['[', '{']);
            step.params[k] = if TEXT_PARAMS.contains(&k) && !structured { json!(v) } else { serde_json::from_str(v).unwrap_or_else(|_| json!(v)) };
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
    // videos, sound and subtitles go into the video, the rest into the photo library
    let (video_imports, imports): (Vec<String>, Vec<String>) = imports.into_iter().partition(|f| br_video::is_video_file(f));
    if !imports.is_empty() {
        import_and_select(&mut b.inner.session, &imports)?;
    }
    let mut video = br_video::Session::default();
    if !video_imports.is_empty() {
        let st = br_video::open_files(&mut video, &video_imports)?;
        // a file that could not be read stops the run, as a photo that could not be imported does
        if let Some(errors) = st.get("errors").filter(|e| e.as_array().is_some_and(|a| !a.is_empty())) {
            return Err(format!("not imported: {errors}"));
        }
    }
    let mut failed = false;
    for step in steps {
        let t = std::time::Instant::now();
        let r = if br_video::is_video(&step.id) {
            br_video::execute(&mut video, &step.id, &step.params)
        } else if let Some(id) = step.id.strip_prefix("film.") {
            // any of FilmCraft's commands, on the video
            br_video::execute(&mut video, id, &step.params)
        } else {
            b.call("engine.execute", json!({"command": step.id, "params": step.params}))
        };
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

/// `br commands`: BR's photo and video commands; `--film` lists FilmCraft's.
fn list_commands(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None => {}
        Some("--film") => {
            for c in filmcraft_engine::command_specs() {
                outln(format!("film.{:<34} {}\n    {}", c.id, c.label, c.params))?;
            }
            return Ok(());
        }
        Some(other) => return Err(format!("unknown option `{other}` (br commands [--film])")),
    }
    outln("Foto (LightCraft):")?;
    for (id, label, params) in commands::COMMANDS {
        outln(format!("  {id:<20} {label}\n  {:<20} {params}", ""))?;
    }
    outln("\nVideo (FilmCraft):")?;
    for c in br_video::COMMANDS {
        outln(format!("  {:<20} {}\n  {:<20} {}", c.id, c.summary, "", c.params))?;
    }
    outln(format!(
        "\nAnd every one of FilmCraft's {} commands as film.<id> (br commands --film), every one of\nLightCraft's by its id (develop.set, library.select, …).",
        filmcraft_engine::command_specs().len()
    ))
}

fn snapshot(args: &[String]) -> Result<(), String> {
    let mut opts = Opts::default();
    let mut out = PathBuf::from("br-snapshot.png");
    let mut size = [1440.0f32, 900.0];
    let mut scale = 1.0f32;
    let mut popover = None;
    let mut page = None;
    let rest = library_opts(args, &mut opts)?;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out = PathBuf::from(it.next().ok_or("-o needs a file")?),
            "--popover" => {
                let v = it.next().ok_or("--popover needs a tool")?;
                popover = Some(br_ui::Popover::ALL.into_iter().find(|p| p.key() == v).ok_or(format!("unknown tool `{v}`: looks, format or export"))?);
            }
            "--page" => {
                let v = it.next().ok_or("--page needs a page")?;
                let keys = br_ui::Page::ALL.map(br_ui::Page::key).join(", ");
                page = Some(br_ui::Page::ALL.into_iter().find(|p| p.key() == v).ok_or(format!("unknown page `{v}`: {keys}"))?);
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
    let (videos, photos): (Vec<String>, Vec<String>) = opts.files.iter().cloned().partition(|f| br_video::is_video_file(f));
    if !photos.is_empty() {
        import_and_select(&mut app.lc.session, &photos)?;
    }
    let mut d = br_ui::headless::Driver::new(size, scale);
    if !videos.is_empty() {
        br_ui::cut::open_files(&mut app, &d.view.ctx.clone(), videos.clone());
        if let Some((text, _)) = app.fc.toast.take().filter(|_| app.fc.session.active_sequence().is_none()) {
            return Err(text);
        }
    }
    // the photo tools are on the Foto page, and so are photos given here (without videos)
    let photos = popover.is_some() || (!photos.is_empty() && videos.is_empty());
    match page {
        Some(p) if popover.is_some() && p != br_ui::Page::Foto => return Err("--popover is a tool of the foto page".into()),
        Some(p) => app.open(p),
        None if photos => app.open(br_ui::Page::Foto),
        None => {}
    }
    d.settle(&mut app, Duration::from_secs(120));
    if let Some(p) = popover {
        // open it the way a click does: under its button in the header
        let button = if p == br_ui::Popover::Export { "export".to_string() } else { format!("tool.{}", p.key()) };
        d.click(&mut app, &button)?;
        d.settle(&mut app, Duration::from_secs(120));
    }
    // FilmCraft renders its monitors on worker threads: wait for the picture
    if app.page.is_video() && app.fc.session.active_sequence().is_some_and(|q| q.duration().0 > 0) {
        let t0 = std::time::Instant::now();
        while app.fc.program_picture().is_none() && t0.elapsed() < Duration::from_secs(15) {
            d.step(&mut app, Vec::new());
            std::thread::sleep(Duration::from_millis(15));
        }
        d.settle(&mut app, Duration::from_secs(30));
    }
    let png = br_ui::headless::png(&d.paint());
    std::fs::write(&out, png).map_err(|e| format!("{}: {e}", out.display()))?;
    app.on_exit();
    outln(out.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--library` first; then the library last opened in the window, unless `$BR_LIBRARY` names
    /// one (as `LIGHTCRAFT_LIBRARY` does for LightCraft); else BR's default (`None` here).
    #[test]
    fn the_window_reopens_the_library_it_had_open() {
        let mut saved = UiState::default();
        saved.settings.library_path = "/Volumes/Photos/Trip".into();
        let cli = || Some(PathBuf::from("/tmp/cli"));
        assert_eq!(launch_library(cli(), Some(&saved), false), cli());
        assert_eq!(launch_library(None, Some(&saved), false), Some(PathBuf::from("/Volumes/Photos/Trip")));
        assert_eq!(launch_library(None, Some(&saved), true), None);
        assert_eq!(launch_library(None, None, false), None);
        // never opened one: BR's default
        saved.settings.library_path = " ".into();
        assert_eq!(launch_library(None, Some(&saved), false), None);
    }

    #[test]
    fn the_panic_report_goes_beside_the_log_or_to_the_temp_folder() {
        let temp = PathBuf::from("/tmp");
        let log = Path::new("/home/anna/.local/share/BR/Logs/br.log");
        assert_eq!(panic_log_path(Some(log), temp.clone()), PathBuf::from("/home/anna/.local/share/BR/Logs/br-panics.log"));
        assert_eq!(panic_log_path(None, temp.clone()), temp.join("br-panics.log"));
    }
}
