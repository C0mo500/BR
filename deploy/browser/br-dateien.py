#!/usr/bin/env python3
"""BR-Dateien: die Seite rund um BR im Browser.

Zeigt BR (noVNC unter ``novnc/``, vom Reverse-Proxy an websockify weitergereicht) und nimmt Fotos
entgegen: hochgeladene Fotos landen im Foto-Ordner, den der Import-Dialog von BR öffnet; Exporte
lassen sich einzeln oder als ZIP herunterladen.

Läuft auf 127.0.0.1 hinter dem Reverse-Proxy, der HTTPS und den Login übernimmt (README.md).
Nur die Python-Standardbibliothek.
"""

import argparse
import json
import mimetypes
import os
import secrets
import shutil
import sys
import urllib.parse
import zipfile
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# was BR (LightCraft) importiert; andere Dateien werden nicht angenommen
PHOTO_EXTENSIONS = {
    "jpg", "jpeg", "png", "tif", "tiff", "webp", "dng", "cr2", "cr3", "nef", "nrw", "arw", "raf",
    "orf", "rw2", "rwl", "raw", "pef", "srw", "psd", "jxl", "gif", "bmp", "heic", "heif", "avif",
    "iiq", "crw", "mrw", "x3f", "kdc", "mos", "erf", "3fr", "fff",
}  # fmt: skip
CHUNK = 1 << 20
# so viel Platz bleibt auf der Platte frei, auch nach einem Upload
RESERVE = 512 << 20
MAX_LISTED = 2000


def safe_name(raw: str) -> str:
    """Der Dateiname ohne Ordner, Steuerzeichen und führende Punkte; leer, wenn nichts bleibt."""
    name = raw.replace("\\", "/").rsplit("/", 1)[-1]
    name = "".join(c for c in name if c.isprintable()).strip().lstrip(".").strip()
    while len(name.encode()) > 200:
        stem, dot, ext = name.rpartition(".")
        name = (stem[:-1] + dot + ext) if stem else name[:-1]
    return name


def extension(name: str) -> str:
    return name.rpartition(".")[2].lower() if "." in name else ""


def place(tmp: Path, folder: Path, name: str) -> Path:
    """Gibt `tmp` den Namen `name` in `folder`, ohne etwas zu überschreiben: „a (2).jpg“ usw."""
    stem, dot, ext = name.rpartition(".")
    for i in range(1, 10_000):
        target = folder / (name if i == 1 else f"{stem} ({i}){dot}{ext}")
        try:
            os.link(tmp, target)
        except FileExistsError:
            continue
        except OSError:
            # ein Dateisystem ohne harte Links
            if target.exists():
                continue
            os.rename(tmp, target)
            return target
        tmp.unlink()
        return target
    raise OSError("kein freier Dateiname")


def visible(name: str) -> bool:
    return not name.startswith(".")


def list_photos(folder: Path) -> list:
    out = []
    with os.scandir(folder) as it:
        for e in it:
            if visible(e.name) and e.is_file(follow_symlinks=False) and extension(e.name) in PHOTO_EXTENSIONS:
                st = e.stat(follow_symlinks=False)
                out.append({"name": e.name, "size": st.st_size, "mtime": int(st.st_mtime)})
    out.sort(key=lambda f: f["mtime"], reverse=True)
    return out


def list_exports(root: Path) -> list:
    out = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames if visible(d))
        for n in filenames:
            p = Path(dirpath, n)
            if not visible(n) or p.is_symlink():
                continue
            st = p.stat()
            out.append({"path": p.relative_to(root).as_posix(), "size": st.st_size, "mtime": int(st.st_mtime)})
    out.sort(key=lambda f: f["mtime"], reverse=True)
    return out


def export_file(root: Path, rel: str):
    """Die Export-Datei `rel` (ein Pfad in `root`), oder None, wenn es sie dort nicht gibt."""
    if "\x00" in rel or not rel:
        return None
    parts = rel.split("/")
    if any(not p or not visible(p) for p in parts):
        return None
    p = root.joinpath(*parts)
    try:
        real = p.resolve(strict=True)
    except OSError:
        return None
    if not real.is_relative_to(root.resolve()) or not real.is_file() or p.is_symlink():
        return None
    return real


def disposition(name: str) -> str:
    ascii_name = "".join(c if c.isascii() and c.isprintable() and c not in '"\\' else "_" for c in name)
    return f"attachment; filename=\"{ascii_name}\"; filename*=UTF-8''{urllib.parse.quote(name)}"


class Handler(BaseHTTPRequestHandler):
    server_version = "BR-Dateien"
    photos: Path
    exports: Path
    max_bytes: int

    def log_message(self, fmt, *args):
        sys.stderr.write("br-dateien: " + (fmt % args) + "\n")

    def reply(self, status: int, body: bytes, ctype: str, extra=None):
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def json(self, status: int, value):
        self.reply(status, json.dumps(value, ensure_ascii=False).encode(), "application/json; charset=utf-8")

    def fail(self, status: int, message: str):
        self.json(status, {"fehler": message})

    def route(self):
        url = urllib.parse.urlsplit(self.path)
        return urllib.parse.unquote(url.path)

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        path = self.route()
        if path == "/":
            self.reply(
                200,
                INDEX.encode(),
                "text/html; charset=utf-8",
                {
                    "Content-Security-Policy": "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; "
                    "img-src 'self' data:; frame-ancestors 'self'",
                    "X-Frame-Options": "SAMEORIGIN",
                },
            )
        elif path == "/api/dateien":
            self.json(
                200,
                {
                    "fotos": list_photos(self.photos)[:MAX_LISTED],
                    "exporte": list_exports(self.exports)[:MAX_LISTED],
                    "max_mb": self.max_bytes >> 20,
                },
            )
        elif path == "/exporte.zip":
            self.zip_exports()
        elif path.startswith("/exporte/"):
            self.send_export(path.removeprefix("/exporte/"))
        else:
            self.fail(404, "Nicht gefunden")

    def send_export(self, rel: str):
        p = export_file(self.exports, rel)
        if p is None:
            return self.fail(404, "Diesen Export gibt es nicht")
        size = p.stat().st_size
        self.send_response(200)
        self.send_header("Content-Type", mimetypes.guess_type(p.name)[0] or "application/octet-stream")
        self.send_header("Content-Length", str(size))
        self.send_header("Content-Disposition", disposition(p.name))
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        if self.command != "HEAD":
            with open(p, "rb") as f:
                shutil.copyfileobj(f, self.wfile, CHUNK)

    def zip_exports(self):
        files = list_exports(self.exports)
        self.send_response(200)
        self.send_header("Content-Type", "application/zip")
        self.send_header("Content-Disposition", disposition("BR Exporte.zip"))
        self.send_header("Connection", "close")
        self.end_headers()
        if self.command == "HEAD":
            return
        # JPEGs packen sich nicht kleiner: gespeichert, nicht komprimiert; ohne Länge vorab
        with zipfile.ZipFile(self.wfile, "w", zipfile.ZIP_STORED, allowZip64=True) as z:
            for f in files:
                p = export_file(self.exports, f["path"])
                if p is not None:
                    z.write(p, f["path"])

    def do_PUT(self):
        path = self.route()
        if not path.startswith("/api/fotos/"):
            return self.fail(404, "Nicht gefunden")
        # nur die eigene Seite lädt hoch: ein anderer Ursprung bekommt diesen Header nicht ohne CORS
        if self.headers.get("X-BR") != "1":
            return self.fail(403, "Nicht erlaubt")
        name = safe_name(path.removeprefix("/api/fotos/"))
        if not name:
            return self.fail(400, "Kein gültiger Dateiname")
        if extension(name) not in PHOTO_EXTENSIONS:
            return self.fail(415, f"„{name}“ ist kein Foto, das BR öffnen kann")
        if self.headers.get("Transfer-Encoding") or self.headers.get("Content-Length") is None:
            return self.fail(411, "Größe fehlt")
        try:
            length = int(self.headers["Content-Length"])
        except ValueError:
            return self.fail(400, "Ungültige Größe")
        if length < 0 or length > self.max_bytes:
            return self.fail(413, f"„{name}“ ist zu groß (höchstens {self.max_bytes >> 20} MB)")
        if shutil.disk_usage(self.photos).free < length + RESERVE:
            return self.fail(507, "Auf dem Server ist nicht genug Platz")
        tmp = self.photos / f".upload-{secrets.token_hex(8)}.part"
        try:
            with open(tmp, "xb") as f:
                left = length
                while left:
                    chunk = self.rfile.read(min(CHUNK, left))
                    if not chunk:
                        raise ConnectionError("Upload abgebrochen")
                    f.write(chunk)
                    left -= len(chunk)
                f.flush()
                os.fsync(f.fileno())
            final = place(tmp, self.photos, name)
        except (OSError, ConnectionError) as e:
            tmp.unlink(missing_ok=True)
            self.close_connection = True
            try:
                self.fail(500, f"„{name}“ wurde nicht gespeichert: {e}")
            except OSError:
                pass  # der Browser ist schon weg
            return
        self.json(201, {"name": final.name, "size": length})


INDEX = r"""<!doctype html>
<html lang="de">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BR Foto</title>
<style>
:root { --bg: #161616; --bar: #232323; --line: #333; --text: #e8e8e8; --dim: #9a9a9a; --accent: #1fc8b4; --bad: #ff6b6b; }
* { box-sizing: border-box; }
html, body { margin: 0; height: 100%; background: var(--bg); color: var(--text); font: 14px/1.4 system-ui, sans-serif; }
body { display: flex; flex-direction: column; overflow: hidden; }
header { height: 44px; flex: none; display: flex; align-items: center; gap: 10px; padding: 0 12px; background: var(--bar); border-bottom: 1px solid var(--line); }
.brand { font-weight: 600; margin-right: 8px; } .brand b { color: var(--accent); }
button, .button { font: inherit; color: var(--text); background: #2e2e2e; border: 1px solid #444; border-radius: 6px; padding: 5px 12px; cursor: pointer; white-space: nowrap; }
button:hover, .button:hover { border-color: var(--accent); }
.primary { background: var(--accent); border-color: var(--accent); color: #062a26; font-weight: 600; }
#status { color: var(--dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; min-width: 0; }
#status.bad { color: var(--bad); }
.count { display: inline-block; min-width: 18px; padding: 0 5px; margin-left: 4px; border-radius: 9px; background: #444; font-size: 12px; text-align: center; }
main { flex: 1; position: relative; min-height: 0; }
iframe { position: absolute; inset: 0; width: 100%; height: 100%; border: 0; background: #1c1c1c; }
aside { position: absolute; top: 0; right: 0; bottom: 0; width: min(380px, 100%); background: var(--bar); border-left: 1px solid var(--line); display: none; flex-direction: column; z-index: 2; }
aside.open { display: flex; }
aside h2 { font-size: 15px; margin: 0; padding: 12px 14px; display: flex; align-items: center; gap: 8px; border-bottom: 1px solid var(--line); }
aside h2 span { flex: 1; }
aside .actions { padding: 10px 14px; display: flex; gap: 8px; border-bottom: 1px solid var(--line); }
#list { overflow: auto; flex: 1; padding: 4px 0; }
#list a { display: flex; gap: 10px; padding: 7px 14px; color: var(--text); text-decoration: none; }
#list a:hover { background: #2c2c2c; }
#list a span:first-child { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
#list a span:last-child, .hint { color: var(--dim); }
.hint { padding: 10px 14px; font-size: 13px; border-top: 1px solid var(--line); }
#drop { position: absolute; inset: 0; z-index: 3; display: none; align-items: center; justify-content: center; background: rgba(31, 200, 180, 0.15); border: 3px dashed var(--accent); font-size: 22px; font-weight: 600; pointer-events: none; }
#drop.on { display: flex; }
@media (max-width: 640px) { .brand span, #full { display: none; } }
</style>
</head>
<body>
<header>
  <div class="brand"><b>BR</b> <span>Foto</span></div>
  <label class="button primary" for="pick">Fotos hochladen</label>
  <input id="pick" type="file" multiple hidden>
  <div id="status"></div>
  <button id="exports">Exporte<span class="count" id="n">0</span></button>
  <button id="full" title="Vollbild">⛶</button>
</header>
<main>
  <iframe id="br" title="BR" allow="clipboard-read; clipboard-write; fullscreen"></iframe>
  <aside id="panel">
    <h2><span>Exporte</span><button id="close" title="Schließen">✕</button></h2>
    <div class="actions"><a class="button" href="exporte.zip">Alle als ZIP</a><button id="reload">Aktualisieren</button></div>
    <div id="list"></div>
    <div class="hint" id="photos"></div>
  </aside>
  <div id="drop">Fotos hier ablegen zum Hochladen</div>
</main>
<script>
"use strict";
const EXT = new Set(__EXTENSIONS__);
const $ = (id) => document.getElementById(id);
const base = location.pathname.replace(/[^/]*$/, "");
const params = new URLSearchParams({ autoconnect: 1, resize: "remote", reconnect: 1, reconnect_delay: 1500, quality: 8, compression: 2,
  path: base.slice(1) + "novnc/websockify" });
$("br").src = base + "novnc/vnc.html?" + params;

function say(text, bad) { $("status").textContent = text; $("status").className = bad ? "bad" : ""; }
function size(n) { return n >= 1e9 ? (n / 1e9).toFixed(1) + " GB" : n >= 1e6 ? (n / 1e6).toFixed(1) + " MB" : Math.max(1, Math.round(n / 1e3)) + " kB"; }
function isPhoto(f) { const i = f.name.lastIndexOf("."); return i > 0 && EXT.has(f.name.slice(i + 1).toLowerCase()); }

async function refresh() {
  try {
    const r = await fetch("api/dateien", { cache: "no-store" });
    const d = await r.json();
    maxBytes = d.max_mb * 1048576;
    $("n").textContent = d.exporte.length;
    const list = $("list");
    list.replaceChildren();
    for (const f of d.exporte) {
      const a = document.createElement("a");
      a.href = "exporte/" + f.path.split("/").map(encodeURIComponent).join("/");
      a.download = f.path.split("/").pop();
      const name = document.createElement("span"), meta = document.createElement("span");
      name.textContent = f.path;
      meta.textContent = size(f.size);
      a.append(name, meta);
      list.append(a);
    }
    if (!d.exporte.length) { const p = document.createElement("div"); p.className = "hint"; p.textContent = "Noch keine Exporte. In BR: Export → Exportieren."; list.append(p); }
    $("photos").textContent = d.fotos.length + " Fotos auf dem Server. Hochgeladene Fotos bleiben dort: BR bearbeitet sie, ohne das Original zu verändern.";
  } catch (e) { say("Die Liste konnte nicht geladen werden.", true); }
}

function put(file, onProgress) {
  return new Promise((resolve, reject) => {
    const x = new XMLHttpRequest();
    x.open("PUT", "api/fotos/" + encodeURIComponent(file.name));
    x.setRequestHeader("X-BR", "1");
    x.upload.onprogress = (e) => e.lengthComputable && onProgress(e.loaded);
    x.onload = () => {
      let d = {}; try { d = JSON.parse(x.responseText); } catch (_) {}
      x.status === 201 ? resolve(d) : reject(new Error(d.fehler || "Fehler " + x.status));
    };
    x.onerror = () => reject(new Error("Verbindung unterbrochen"));
    x.send(file);
  });
}

let uploading = false, maxBytes = Infinity;
async function upload(files) {
  if (uploading) { say("Es wird schon hochgeladen …"); return; }
  const photos = [...files].filter(isPhoto), skipped = files.length - photos.length;
  if (!photos.length) { say(skipped ? "Keine Fotos dabei – BR öffnet JPEG, PNG, TIFF, HEIC und Raw-Dateien." : "", !!skipped); return; }
  uploading = true;
  const total = photos.reduce((s, f) => s + f.size, 0);
  let done = 0, ok = 0; const failed = [];
  for (const [i, f] of photos.entries()) {
    try {
      if (f.size > maxBytes) throw new Error(`zu groß (höchstens ${maxBytes / 1048576} MB)`);
      await put(f, (n) => say(`Lade hoch: ${i + 1} von ${photos.length} · ${Math.floor(((done + n) / Math.max(total, 1)) * 100)} %`));
      ok++;
    } catch (e) { failed.push(f.name + ": " + e.message); }
    done += f.size;
  }
  uploading = false;
  if (failed.length) say(`${ok} hochgeladen, ${failed.length} nicht: ${failed.join(" · ")}`, true);
  else say(`${ok} ${ok === 1 ? "Foto" : "Fotos"} hochgeladen${skipped ? ` (${skipped} ${skipped === 1 ? "Datei" : "Dateien"} ohne Foto übersprungen)` : ""}. In BR: „Importieren“ → Strg+A → OK.`);
  refresh();
}

$("pick").addEventListener("change", (e) => { upload(e.target.files); e.target.value = ""; });
$("exports").onclick = () => { $("panel").classList.toggle("open"); refresh(); };
$("close").onclick = () => $("panel").classList.remove("open");
$("reload").onclick = refresh;
$("full").onclick = () => document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen();

// Dateien auf die Seite ziehen – auch auf das BR-Fenster (die noVNC-Seite hat denselben Ursprung)
let depth = 0;
function watchDrops(target) {
  const hasFiles = (e) => [...(e.dataTransfer?.types || [])].includes("Files");
  target.addEventListener("dragenter", (e) => { if (hasFiles(e)) { depth++; $("drop").classList.add("on"); e.preventDefault(); } });
  target.addEventListener("dragover", (e) => { if (hasFiles(e)) e.preventDefault(); });
  target.addEventListener("dragleave", () => { depth = Math.max(0, depth - 1); if (!depth) $("drop").classList.remove("on"); });
  target.addEventListener("drop", (e) => { if (!hasFiles(e)) return; e.preventDefault(); depth = 0; $("drop").classList.remove("on"); upload(e.dataTransfer.files); });
}
watchDrops(window);
$("br").addEventListener("load", () => { try { watchDrops($("br").contentWindow); } catch (_) {} });
refresh();
setInterval(() => { if (!document.hidden) refresh(); }, 8000);
</script>
</body>
</html>
""".replace("__EXTENSIONS__", json.dumps(sorted(PHOTO_EXTENSIONS)))


def main():
    a = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    a.add_argument("--fotos", type=Path, default=Path(os.environ.get("BR_IMPORT_DIR", "/srv/br/Fotos")), help="Ordner für hochgeladene Fotos")
    a.add_argument("--exporte", type=Path, default=Path(os.environ.get("BR_EXPORTS", "/srv/br/Exporte")), help="der Export-Ordner von BR")
    a.add_argument("--bind", default="127.0.0.1")
    a.add_argument("--port", type=int, default=int(os.environ.get("BR_FILES_PORT", "6081")))
    a.add_argument("--max-mb", type=int, default=int(os.environ.get("BR_UPLOAD_MAX_MB", "2048")), help="größte Datei in MB")
    args = a.parse_args()
    for d in (args.fotos, args.exporte):
        d.mkdir(parents=True, exist_ok=True)
    Handler.photos, Handler.exports, Handler.max_bytes = args.fotos.resolve(), args.exporte.resolve(), args.max_mb << 20
    server = ThreadingHTTPServer((args.bind, args.port), Handler)
    server.daemon_threads = True
    print(f"br-dateien: http://{args.bind}:{args.port}/ – Fotos {Handler.photos}, Exporte {Handler.exports}", file=sys.stderr)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
