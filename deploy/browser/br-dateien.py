#!/usr/bin/env python3
"""BR-Dateien: die Seite rund um BR im Browser.

Zeigt BR (noVNC unter ``novnc/``, vom Reverse-Proxy an websockify weitergereicht) und nimmt Fotos,
Videos, Ton und Untertitel entgegen: hochgeladene Dateien landen im Foto-Ordner, den der Import-Dialog von BR öffnet; Exporte
lassen sich einzeln oder als ZIP herunterladen. Das VNC-Passwort, das br-web bei jedem Start neu
setzt, gibt es nur der eigenen Seite (``/api/vnc``).

Auf das BR-Fenster gezogene Dateien legt die Seite nach dem Hochladen dort ab, wo sie losgelassen
wurden (``/api/ablegen`` → BRs Steuerkanal, ``br.drop``). Den Ton der Wiedergabe spielt BR in eine
Socket-Datei (``BR_AUDIO_STREAM``); ``/api/ton`` reicht ihn an die Seite, die ihn mit einem
AudioWorklet (``ton.js``) abspielt.

Läuft auf 127.0.0.1 hinter dem Reverse-Proxy, der HTTPS und den Login übernimmt (README.md).
Nur die Python-Standardbibliothek.
"""

import argparse
import json
import mimetypes
import os
import secrets
import shutil
import socket
import sys
import threading
import urllib.parse
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# was BR (LightCraft) importiert; andere Dateien werden nicht angenommen
PHOTO_EXTENSIONS = {
    "jpg", "jpeg", "png", "tif", "tiff", "webp", "dng", "cr2", "cr3", "nef", "nrw", "arw", "raf",
    "orf", "rw2", "rwl", "raw", "pef", "srw", "psd", "jxl", "gif", "bmp", "heic", "heif", "avif",
    "iiq", "crw", "mrw", "x3f", "kdc", "mos", "erf", "3fr", "fff",
}  # fmt: skip
# was BR (FilmCraft) für Videos nimmt: Video, Ton, Untertitel
VIDEO_EXTENSIONS = {
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mxf", "mts", "m2ts", "ts", "mpg", "mpeg", "3gp",
    "mp3", "m4a", "aac", "wav", "aif", "aiff", "flac", "ogg", "opus",
    "srt", "vtt", "scc",
}  # fmt: skip
MEDIA_EXTENSIONS = PHOTO_EXTENSIONS | VIDEO_EXTENSIONS
CHUNK = 1 << 20
# so viele Ton-Verbindungen zugleich (je eine pro offener Seite mit eingeschaltetem Ton)
MAX_TON = 4
# ohne Ton von BR (nichts spielt) alle so viele Sekunden ein stilles Sample-Paar, damit eine
# geschlossene Seite bemerkt wird
TON_KEEPALIVE = 5
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


def list_photos(folder: Path, kinds=PHOTO_EXTENSIONS) -> list:
    out = []
    with os.scandir(folder) as it:
        for e in it:
            if visible(e.name) and e.is_file(follow_symlinks=False) and extension(e.name) in kinds:
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


def control_file() -> Path:
    """Wo BR Port und Token seines Steuerkanals ablegt (wie BRs Einstellungsordner)."""
    d = os.environ.get("BR_CONFIG_DIR")
    if d:
        return Path(d) / "control.json"
    base = os.environ.get("XDG_CONFIG_HOME", "")
    return (Path(base) if os.path.isabs(base) else Path.home() / ".config") / "BR" / "control.json"


def br_request(info: Path, method: str, params: dict, timeout: float = 20) -> dict:
    """Eine Anfrage an das BR-Fenster über seinen Steuerkanal (mit dem Token aus `info`)."""
    data = json.loads(info.read_text())
    with socket.create_connection(("127.0.0.1", int(data["port"])), timeout=timeout) as s:
        f = s.makefile("rwb")

        def call(i, m, p):
            f.write((json.dumps({"id": i, "method": m, "params": p}) + "\n").encode())
            f.flush()
            line = f.readline(1 << 20)
            if not line:
                raise ConnectionError("BR hat die Verbindung geschlossen")
            return json.loads(line)

        if not call(1, "auth", {"token": data["token"]}).get("ok"):
            raise PermissionError("BR hat das Token abgelehnt")
        return call(2, method, params)


def disposition(name: str) -> str:
    ascii_name = "".join(c if c.isascii() and c.isprintable() and c not in '"\\' else "_" for c in name)
    return f"attachment; filename=\"{ascii_name}\"; filename*=UTF-8''{urllib.parse.quote(name)}"


class Handler(BaseHTTPRequestHandler):
    server_version = "BR-Dateien"
    photos: Path
    exports: Path
    max_bytes: int
    vnc_password_file: Path
    ton_socket: Path = Path("/nonexistent")
    control_info: Path = Path("/nonexistent")
    ton_slots = threading.BoundedSemaphore(MAX_TON)

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
        # ein Dateiname, der kein UTF-8 ist (aus einem alten ZIP), soll nicht die ganze Liste kosten
        self.reply(status, json.dumps(value, ensure_ascii=False).encode("utf-8", "replace"), "application/json; charset=utf-8")

    def fail(self, status: int, message: str):
        self.json(status, {"fehler": message})

    def own_page(self) -> bool:
        """Kommt die Anfrage von der eigenen Seite? Eine fremde Seite kann diesen Header nicht ohne
        CORS setzen, und CORS erlaubt br-dateien nicht."""
        return self.headers.get("X-BR") == "1"

    def refuse(self, status: int, message: str):
        """Eine Absage auf einen Upload, dessen Inhalt noch unterwegs ist: erst antworten, dann den
        Rest lesen und verwerfen. Sonst setzt der Kernel die Verbindung zurück, und der Proxy macht
        aus der Absage ein „502“."""
        self.close_connection = True
        self.fail(status, message)
        try:
            self.wfile.flush()
            self.connection.shutdown(socket.SHUT_WR)
            self.connection.settimeout(60)
            while self.rfile.read(CHUNK):
                pass
        except OSError:
            pass

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
            exports = list_exports(self.exports)
            self.json(
                200,
                {
                    "fotos": sum(1 for _ in list_photos(self.photos)),
                    "videos": sum(1 for _ in list_photos(self.photos, VIDEO_EXTENSIONS)),
                    "exporte": exports[:MAX_LISTED],
                    "exporte_gesamt": len(exports),
                    "max_mb": self.max_bytes >> 20,
                },
            )
        elif path == "/api/vnc":
            self.vnc_password()
        elif path == "/api/ton":
            self.ton()
        elif path == "/ton.js":
            self.reply(200, TON_JS.encode(), "text/javascript; charset=utf-8")
        elif path == "/exporte.zip":
            self.zip_exports()
        elif path.startswith("/exporte/"):
            self.send_export(path.removeprefix("/exporte/"))
        else:
            self.fail(404, "Nicht gefunden")

    def vnc_password(self):
        if not self.own_page():
            return self.fail(403, "Nicht erlaubt")
        try:
            password = self.vnc_password_file.read_text().strip()
        except OSError:
            password = ""
        if not password:
            return self.fail(503, "BR startet gerade")
        self.json(200, {"passwort": password})

    def ton(self):
        """Der Ton der Wiedergabe, wie BR ihn in die Socket-Datei spielt: PCM, 16 Bit, 48 kHz,
        Stereo, ohne Ende."""
        if not self.own_page():
            return self.fail(403, "Nicht erlaubt")
        if not self.ton_slots.acquire(blocking=False):
            return self.fail(429, "Zu viele Seiten mit Ton")
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                s.connect(str(self.ton_socket))
            except OSError:
                s.close()
                return self.fail(503, "Kein Ton: BR spielt nicht in den Browser (BR_TON in /etc/br-web.env)")
            self.close_connection = True
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Accel-Buffering", "no")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("X-Ton", "s16le; rate=48000; channels=2")
            self.end_headers()
            with s:
                s.settimeout(TON_KEEPALIVE)
                while True:
                    try:
                        chunk = s.recv(1 << 16)
                    except TimeoutError:
                        chunk = b"\0\0\0\0"
                    if not chunk:
                        break
                    self.wfile.write(chunk)
                    self.wfile.flush()
        except OSError:
            pass  # die Seite ist zu, oder BR hat neu gestartet
        finally:
            self.ton_slots.release()

    def do_POST(self):
        path = self.route()
        if path != "/api/ablegen":
            return self.refuse(404, "Nicht gefunden")
        if not self.own_page():
            return self.refuse(403, "Nicht erlaubt")
        try:
            length = int(self.headers.get("Content-Length", ""))
        except ValueError:
            return self.refuse(411, "Größe fehlt")
        if not 0 < length <= 1 << 16:
            return self.refuse(413, "Zu groß")
        try:
            body = json.loads(self.rfile.read(length))
            names, x, y = body["dateien"], body.get("x"), body.get("y")
            assert isinstance(names, list) and names
        except (ValueError, KeyError, TypeError, AssertionError):
            return self.fail(400, "Erwartet: {dateien: [Name, …], x, y}")
        paths = []
        for n in names:
            if not isinstance(n, str) or safe_name(n) != n or not (self.photos / n).is_file():
                return self.fail(404, f"„{n}“ ist nicht hochgeladen")
            paths.append(str(self.photos / n))
        params = {"paths": paths}
        if all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in (x, y)):
            params["screen"] = [float(x), float(y)]
        try:
            r = br_request(self.control_info, "br.drop", params)
        except (OSError, ValueError, KeyError, PermissionError) as e:
            return self.fail(503, f"BR ist nicht erreichbar ({e}); BR_CONTROL=1 in /etc/br-web.env?")
        if not r.get("ok"):
            return self.fail(422, str(r.get("error") or "BR hat abgelehnt"))
        self.json(200, r.get("result"))

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
                    z.write(p, f["path"].encode("utf-8", "replace").decode())

    def do_PUT(self):
        path = self.route()
        if not path.startswith("/api/fotos/"):
            return self.refuse(404, "Nicht gefunden")
        if not self.own_page():
            return self.refuse(403, "Nicht erlaubt")
        name = safe_name(path.removeprefix("/api/fotos/"))
        if not name:
            return self.refuse(400, "Kein gültiger Dateiname")
        if extension(name) not in MEDIA_EXTENSIONS:
            return self.refuse(415, f"„{name}“ ist weder Foto noch Video, das BR öffnen kann")
        if self.headers.get("Transfer-Encoding") or self.headers.get("Content-Length") is None:
            return self.refuse(411, "Größe fehlt")
        try:
            length = int(self.headers["Content-Length"])
        except ValueError:
            return self.refuse(400, "Ungültige Größe")
        if length < 0 or length > self.max_bytes:
            return self.refuse(413, f"„{name}“ ist zu groß (höchstens {self.max_bytes >> 20} MB)")
        if shutil.disk_usage(self.photos).free < length + RESERVE:
            return self.refuse(507, "Auf dem Server ist nicht genug Platz")
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
            try:
                self.refuse(500, f"„{name}“ wurde nicht gespeichert: {e}")
            except OSError:
                pass  # der Browser ist schon weg
            return
        self.json(201, {"name": final.name, "size": length})


INDEX = r"""<!doctype html>
<html lang="de">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BR</title>
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
main { flex: 1; position: relative; min-height: 0; overflow: auto; }
/* nie kleiner als das BR-Fenster mindestens ist (1000 × 640): sonst ragt BR über den Bildschirm */
iframe { position: absolute; inset: 0; width: 100%; height: 100%; min-width: 1000px; min-height: 640px; border: 0; background: #1c1c1c; }
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
  <div class="brand"><b>BR</b> <span>Foto · Video</span></div>
  <label class="button primary" for="pick" title="Fotos, Videos, Musik und Untertitel">Hochladen</label>
  <input id="pick" type="file" multiple hidden>
  <div id="status"></div>
  <button id="ton" title="Den Ton der Wiedergabe hier hören">🔇 Ton</button>
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
  <div id="drop">Fotos und Videos hier ablegen zum Hochladen</div>
</main>
<script>
"use strict";
// die Seite rechnet mit ihrem Ordner als Adresse: …/br/ statt …/br
if (!location.pathname.endsWith("/")) location.replace(location.pathname + "/" + location.search + location.hash);
const EXT = new Set(__EXTENSIONS__);
const $ = (id) => document.getElementById(id);
const base = location.pathname.replace(/[^/]*$/, "");
// absolute Adressen ohne Zugangsdaten: wer die Seite als https://name:passwort@… öffnet, dem
// verweigert fetch() sonst jede relative Adresse
const here = location.origin + base;
const LIST_ERR = "Die Liste konnte nicht geladen werden.";

function say(text, bad) { $("status").textContent = text; $("status").className = bad ? "bad" : ""; }
function size(n) { return n >= 1e9 ? (n / 1e9).toFixed(1) + " GB" : n >= 1e6 ? (n / 1e6).toFixed(1) + " MB" : Math.max(1, Math.round(n / 1e3)) + " kB"; }
function isPhoto(f) { const i = f.name.lastIndexOf("."); return !f.name.startsWith(".") && i > 0 && EXT.has(f.name.slice(i + 1).toLowerCase()); }

// BR: noVNC im Rahmen. Das VNC-Passwort (neu bei jedem Start von BR) holt die Seite hier ab und
// gibt es noVNC im #-Teil der Adresse mit – der geht an keinen Server und in kein Log.
let loads = 0;
async function connect() {
  let password = "";
  try {
    const r = await fetch(here + "api/vnc", { cache: "no-store", headers: { "X-BR": "1" } });
    if (r.ok) password = (await r.json()).passwort;
  } catch (_) {}
  if (!password) return;
  // alle Verbindungs-Einstellungen festlegen: noVNC 1.0 merkt sich Werte aus einer Adresse, die
  // jemand anderes geöffnet haben könnte (etwa host= eines fremden Servers)
  const https = location.protocol === "https:";
  const p = new URLSearchParams({ autoconnect: 1, resize: "remote", reconnect: 1, reconnect_delay: 2000, quality: 8, compression: 2,
    host: location.hostname, port: location.port || (https ? 443 : 80), encrypt: https ? 1 : 0, shared: 1, view_only: 0, repeaterID: "",
    logging: "warn", view_clip: 0, path: base.slice(1) + "novnc/websockify", password });
  // ersetzen statt neu laden: sonst landet jedes Wiederverbinden im Verlauf, und „Zurück“ verließe BR nicht
  $("br").contentWindow.location.replace(`${here}novnc/vnc.html?n=${++loads}#${p}`);
}
// noVNC versucht nach einem Abbruch nur einmal neu zu verbinden. Ist BR neu gestartet (neues
// Passwort), der Laptop aufgewacht oder das Netz zurück, lädt die Seite noVNC selbst neu.
// Bleibt noVNC beim Laden hängen (ein Fehler beim Start), versucht sie es nach einer Minute neu.
const LIVE = ["noVNC_connected", "noVNC_connecting", "noVNC_reconnecting", "noVNC_disconnecting"];
let down = 0, loading = 0;
setInterval(() => {
  if (document.hidden) return;
  let cls = null;
  try { cls = $("br").contentDocument?.documentElement?.classList; } catch (_) { return; }
  loading = cls?.contains("noVNC_loading") ? loading + 1 : 0;
  down = cls && (LIVE.some((c) => cls.contains(c)) || (loading && loading < 20)) ? 0 : down + 1;
  if (down >= 2) { down = 0; loading = 0; connect(); }
}, 3000);

async function refresh() {
  try {
    const r = await fetch(here + "api/dateien", { cache: "no-store" });
    if (!r.ok) throw new Error(r.status);
    const d = await r.json();
    maxBytes = d.max_mb * 1048576;
    $("n").textContent = d.exporte_gesamt;
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
    const note = (text) => { const p = document.createElement("div"); p.className = "hint"; p.textContent = text; list.append(p); };
    if (!d.exporte.length) note("Noch keine Exporte. In BR: „Exportieren“ oben rechts.");
    else if (d.exporte_gesamt > d.exporte.length) note(`Die neuesten ${d.exporte.length} von ${d.exporte_gesamt}; alle zusammen gibt es als ZIP.`);
    $("photos").textContent = `${d.fotos} Fotos und ${d.videos} Videos auf dem Server. Hochgeladenes bleibt dort: BR bearbeitet es, ohne das Original zu verändern.`;
    if ($("status").textContent === LIST_ERR) say("");
  } catch (e) {
    if (!uploading && (!$("status").textContent || $("status").textContent === LIST_ERR)) say(LIST_ERR, true);
  }
}

function put(file, onProgress) {
  return new Promise((resolve, reject) => {
    const x = new XMLHttpRequest();
    x.open("PUT", here + "api/fotos/" + encodeURIComponent(file.name));
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

let uploading = false, maxBytes = Infinity, queue = [];
// `sum`: was die Runden davor geschafft haben – die Bilanz kommt erst, wenn nichts mehr wartet.
// `at`: wo auf dem BR-Bildschirm die Dateien losgelassen wurden; dort legt BR sie danach ab.
async function upload(files, sum = { ok: 0, failed: [], skipped: 0 }, at = null) {
  files = [...files];
  if (uploading) { queue.push({ files, at }); say(`${queue.reduce((n, q) => n + q.files.length, 0)} weitere Dateien warten …`); return; }
  const photos = files.filter(isPhoto), skipped = sum.skipped + files.length - photos.length;
  if (!photos.length && !sum.ok && !sum.failed.length) {
    say(skipped ? "Weder Fotos noch Videos dabei – BR öffnet JPEG, PNG, HEIC, Raw, MP4, MOV, MKV, Musik und SRT." : "", !!skipped);
    return;
  }
  uploading = true;
  const total = photos.reduce((s, f) => s + f.size, 0);
  let done = 0, ok = sum.ok; const failed = sum.failed, names = [];
  for (const [i, f] of photos.entries()) {
    try {
      if (f.size > maxBytes) throw new Error(`zu groß (höchstens ${maxBytes / 1048576} MB)`);
      const waiting = () => (queue.length ? ` · ${queue.reduce((n, q) => n + q.files.length, 0)} weitere warten` : "");
      const d = await put(f, (n) => say(`Lade hoch: ${i + 1} von ${photos.length} · ${Math.floor(((done + n) / Math.max(total, 1)) * 100)} %${waiting()}`));
      names.push(d.name);
      ok++;
    } catch (e) { failed.push(f.name + ": " + e.message); }
    done += f.size;
  }
  // auf BR gezogen: dort ablegen, wo losgelassen (Timeline, Monitor, Startbildschirm, Foto)
  let placed = "";
  if (at && names.length) {
    try {
      const r = await fetch(here + "api/ablegen", { method: "POST", headers: { "X-BR": "1", "Content-Type": "application/json" }, body: JSON.stringify({ dateien: names, x: at[0], y: at[1] }) });
      const d = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(d.fehler || r.status);
      placed = " und in BR abgelegt";
    } catch (e) { failed.push("Ablegen in BR: " + e.message); }
  }
  uploading = false;
  if (queue.length) { const next = queue.shift(); return upload(next.files, { ok, failed, skipped, placed: sum.placed || placed }, next.at); }
  placed = placed || sum.placed || "";
  if (failed.length) say(`${ok} hochgeladen${placed}, ${failed.length} nicht: ${failed.join(" · ")}`, true);
  else say(`${ok} ${ok === 1 ? "Datei" : "Dateien"} hochgeladen${placed}${skipped ? ` (${skipped} andere übersprungen)` : ""}.${placed ? "" : " In BR: Videos mit „+ Medien“ auf „Schnitt“, Fotos auf „Foto“ mit Importieren."}`);
  refresh();
}

// Wo auf dem BR-Bildschirm ein Ereignis im noVNC-Rahmen liegt (in Bildschirm-Pixeln), oder null.
function screenAt(e) {
  try {
    const doc = $("br").contentDocument;
    const canvas = doc.querySelector("#noVNC_container canvas") || doc.querySelector("canvas");
    if (!canvas || !canvas.width) return null;
    const r = canvas.getBoundingClientRect();
    if (e.clientX < r.left || e.clientY < r.top || e.clientX >= r.right || e.clientY >= r.bottom) return null;
    return [((e.clientX - r.left) * canvas.width) / r.width, ((e.clientY - r.top) * canvas.height) / r.height];
  } catch (_) { return null; }
}

// Der Ton der Wiedergabe: BR spielt ihn auf dem Server, die Seite holt ihn als PCM-Strom
// (api/ton) und spielt ihn mit einem AudioWorklet (ton.js), das die Verzögerung kurz hält.
let ton = null;
async function toggleTon() {
  if (ton) { ton.stop(); return; }
  let ctx;
  try {
    ctx = new AudioContext({ sampleRate: 48000, latencyHint: "interactive" });
    await ctx.audioWorklet.addModule(here + "ton.js");
  } catch (e) { ctx?.close(); say("Ton geht in diesem Browser nicht: " + e.message, true); return; }
  const node = new AudioWorkletNode(ctx, "br-ton", { outputChannelCount: [2] });
  node.connect(ctx.destination);
  const abort = new AbortController();
  let on = true;
  ton = { stop() { on = false; abort.abort(); ctx.close(); ton = null; $("ton").textContent = "🔇 Ton"; } };
  $("ton").textContent = "🔊 Ton";
  while (on) {
    try {
      const r = await fetch(here + "api/ton", { headers: { "X-BR": "1" }, cache: "no-store", signal: abort.signal });
      if (!r.ok) throw new Error((await r.json().catch(() => ({}))).fehler || "Fehler " + r.status);
      if ($("status").textContent.startsWith("Ton:")) say("");
      const reader = r.body.getReader();
      let rest = new Uint8Array(0);
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        let bytes = value;
        if (rest.length) { bytes = new Uint8Array(rest.length + value.length); bytes.set(rest); bytes.set(value, rest.length); }
        // ganze Sample-Paare (4 Bytes); der Rest kommt mit dem nächsten Stück
        const n = bytes.length - (bytes.length % 4);
        rest = bytes.slice(n);
        if (n) { const block = bytes.slice(0, n); node.port.postMessage(block.buffer, [block.buffer]); }
      }
    } catch (e) { if (!on) return; say("Ton: " + e.message, true); }
    if (on) await new Promise((ok) => setTimeout(ok, 2000));
  }
}

// Gezogene Ordner (eine Speicherkarte, ein Kamera-Ordner) werden durchsucht. Die Einträge müssen
// noch während des Ablegens geholt werden, danach sind sie weg.
function dropped(dt) {
  const entries = [...(dt.items || [])].map((i) => i.webkitGetAsEntry?.()).filter(Boolean);
  if (!entries.some((e) => e.isDirectory)) return Promise.resolve([...dt.files]);
  const out = [];
  const walk = async (e) => {
    if (e.isFile) out.push(await new Promise((ok, fail) => e.file(ok, fail)));
    else if (e.isDirectory && !e.name.startsWith(".")) {
      const reader = e.createReader();
      for (;;) {
        const batch = await new Promise((ok, fail) => reader.readEntries(ok, fail));
        if (!batch.length) break;
        for (const c of batch) await walk(c);
      }
    }
  };
  return (async () => { for (const e of entries) await walk(e); return out; })();
}

$("pick").addEventListener("change", (e) => { upload(e.target.files); e.target.value = ""; });
$("exports").onclick = () => { $("panel").classList.toggle("open"); refresh(); };
$("close").onclick = () => $("panel").classList.remove("open");
$("reload").onclick = refresh;
$("ton").onclick = toggleTon;
$("full").onclick = () => document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen();

// Dateien auf die Seite ziehen – auch auf das BR-Fenster (die noVNC-Seite hat denselben Ursprung):
// dort legt BR sie nach dem Hochladen an der Stelle ab, an der sie losgelassen wurden
let depth = 0;
function watchDrops(target, onBr) {
  const hasFiles = (e) => [...(e.dataTransfer?.types || [])].includes("Files");
  target.addEventListener("dragenter", (e) => { if (hasFiles(e)) { depth++; $("drop").classList.add("on"); e.preventDefault(); } });
  target.addEventListener("dragover", (e) => { if (hasFiles(e)) e.preventDefault(); });
  target.addEventListener("dragleave", () => { depth = Math.max(0, depth - 1); if (!depth) $("drop").classList.remove("on"); });
  target.addEventListener("drop", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth = 0;
    $("drop").classList.remove("on");
    const at = onBr ? screenAt(e) : null;
    dropped(e.dataTransfer).then((files) => upload(files, undefined, at), () => say("Der Ordner ließ sich nicht lesen.", true));
  });
}
watchDrops(window, false);
$("br").addEventListener("load", () => { try { watchDrops($("br").contentWindow, true); } catch (_) {} });
connect();
refresh();
setInterval(() => { if (!document.hidden) refresh(); }, 8000);
</script>
</body>
</html>
""".replace("__EXTENSIONS__", json.dumps(sorted(MEDIA_EXTENSIONS)))


TON_JS = r"""// BR: der Ton der Wiedergabe, als PCM-Strom von api/ton (16 Bit, 48 kHz, Stereo)
class BrTon extends AudioWorkletProcessor {
  constructor() {
    super();
    this.size = sampleRate * 2;
    this.l = new Float32Array(this.size);
    this.r = new Float32Array(this.size);
    this.read = 0;
    this.write = 0;
    this.playing = false;
    this.port.onmessage = (e) => {
      const s = new Int16Array(e.data);
      for (let i = 0; i + 1 < s.length; i += 2) {
        this.l[this.write] = s[i] / 32768;
        this.r[this.write] = s[i + 1] / 32768;
        this.write = (this.write + 1) % this.size;
        if (this.write === this.read) this.read = (this.read + 1) % this.size;
      }
      // mehr als 0,35 s im Puffer (das Netz hat gestockt): auf 0,12 s kürzen, damit Bild und Ton
      // beieinander bleiben
      if (this.queued() > 0.35 * sampleRate) this.read = (this.write - Math.round(0.12 * sampleRate) + this.size) % this.size;
    };
  }
  queued() { return (this.write - this.read + this.size) % this.size; }
  process(_inputs, outputs) {
    const L = outputs[0][0], R = outputs[0][1] || outputs[0][0];
    // nach dem Leerlaufen erst wieder ein kleines Polster sammeln
    if (!this.playing && this.queued() < 0.06 * sampleRate) { L.fill(0); R.fill(0); return true; }
    this.playing = true;
    for (let i = 0; i < L.length; i++) {
      if (this.read === this.write) { L[i] = 0; R[i] = 0; this.playing = false; continue; }
      L[i] = this.l[this.read];
      R[i] = this.r[this.read];
      this.read = (this.read + 1) % this.size;
    }
    return true;
  }
}
registerProcessor("br-ton", BrTon);
"""


def main():
    a = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    a.add_argument("--fotos", type=Path, default=Path(os.environ.get("BR_IMPORT_DIR", "/srv/br/Fotos")), help="Ordner für hochgeladene Fotos und Videos")
    a.add_argument("--exporte", type=Path, default=Path(os.environ.get("BR_EXPORTS", "/srv/br/Exporte")), help="der Export-Ordner von BR")
    a.add_argument("--bind", default="127.0.0.1")
    a.add_argument("--port", type=int, default=int(os.environ.get("BR_FILES_PORT", "6081")))
    a.add_argument("--max-mb", type=int, default=int(os.environ.get("BR_UPLOAD_MAX_MB", "2048")), help="größte Datei in MB")
    a.add_argument(
        "--vnc-passwort",
        type=Path,
        default=Path(os.environ.get("BR_VNC_PASSWORD_FILE") or Path.home() / ".br-web" / "vnc-passwort"),
        help="die Datei, in die br-web das VNC-Passwort schreibt",
    )
    a.add_argument(
        "--ton",
        type=Path,
        default=Path(os.environ.get("BR_AUDIO_STREAM") or Path.home() / ".br-web" / "ton.sock"),
        help="die Socket-Datei, in die BR den Ton spielt",
    )
    args = a.parse_args()
    for d in (args.fotos, args.exporte):
        d.mkdir(parents=True, exist_ok=True)
    Handler.photos, Handler.exports, Handler.max_bytes = args.fotos.resolve(), args.exporte.resolve(), args.max_mb << 20
    Handler.vnc_password_file = args.vnc_passwort
    Handler.ton_socket = args.ton
    Handler.control_info = control_file()
    server = ThreadingHTTPServer((args.bind, args.port), Handler)
    server.daemon_threads = True
    print(f"br-dateien: http://{args.bind}:{args.port}/ – Fotos {Handler.photos}, Exporte {Handler.exports}", file=sys.stderr)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
