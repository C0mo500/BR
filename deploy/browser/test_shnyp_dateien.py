"""Tests für shnyp-dateien.py:  python3 -m unittest discover -s deploy/browser"""

import http.client
import importlib.util
import io
import json
import os
import shutil
import socket
import tempfile
import threading
import unittest
import urllib.parse
import zipfile
from http.server import ThreadingHTTPServer
from pathlib import Path

spec = importlib.util.spec_from_file_location("shnyp_dateien", Path(__file__).with_name("shnyp-dateien.py"))
shnyp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shnyp)


class Server(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="shnyp-dateien-"))
        self.photos, self.exports = self.dir / "Fotos", self.dir / "Exporte"
        self.photos.mkdir()
        self.exports.mkdir()

        class Handler(shnyp.Handler):
            pass

        Handler.photos, Handler.exports, Handler.max_bytes = self.photos.resolve(), self.exports.resolve(), 1 << 20
        Handler.vnc_password_file = self.dir / "vnc-passwort"
        Handler.log_message = lambda *a: None
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        shutil.rmtree(self.dir)

    def request(self, method, path, body=None, headers=None):
        c = http.client.HTTPConnection("127.0.0.1", self.server.server_address[1], timeout=10)
        c.request(method, path, body=body, headers=headers or {})
        r = c.getresponse()
        data = r.read()
        c.close()
        return r, data

    def upload(self, name, body=b"\xff\xd8\xff photo", headers=None):
        h = {"X-Shnyp": "1"} if headers is None else headers
        return self.request("PUT", "/api/fotos/" + urllib.parse.quote(name, safe=""), body, h)

    def headers_only(self, name, length):
        """A PUT that announces `length` bytes and sends none: the reply comes before the body."""
        c = http.client.HTTPConnection("127.0.0.1", self.server.server_address[1], timeout=10)
        c.putrequest("PUT", "/api/fotos/" + urllib.parse.quote(name, safe=""))
        c.putheader("X-Shnyp", "1")
        c.putheader("Content-Length", str(length))
        c.endheaders()
        r = c.getresponse()
        r.read()
        c.close()
        return r

    def test_page_and_listing(self):
        r, body = self.request("GET", "/")
        self.assertEqual(r.status, 200)
        self.assertIn(b"novnc/vnc.html", body)
        self.assertEqual(r.getheader("X-Frame-Options"), "SAMEORIGIN")
        (self.exports / "sub").mkdir()
        (self.exports / "sub" / "b.jpg").write_bytes(b"b")
        (self.exports / ".versteckt.jpg").write_bytes(b"x")
        (self.photos / "a.jpg").write_bytes(b"a")
        (self.photos / "clip.MOV").write_bytes(b"v")
        (self.photos / "notiz.txt").write_bytes(b"n")
        d = json.loads(self.request("GET", "/api/dateien")[1])
        self.assertEqual((d["fotos"], d["videos"]), (1, 1))
        self.assertEqual([f["path"] for f in d["exporte"]], ["sub/b.jpg"])
        self.assertEqual((d["exporte_gesamt"], d["max_mb"]), (1, 1))

    def test_more_exports_than_listed(self):
        for i in range(shnyp.MAX_LISTED + 5):
            (self.exports / f"{i}.jpg").write_bytes(b"x")
        d = json.loads(self.request("GET", "/api/dateien")[1])
        self.assertEqual((len(d["exporte"]), d["exporte_gesamt"]), (shnyp.MAX_LISTED, shnyp.MAX_LISTED + 5))

    def test_a_name_that_is_not_utf8_does_not_break_the_lists(self):
        os.close(os.open(os.fsencode(self.photos) + b"/Urlaub_\xe4.jpg", os.O_CREAT | os.O_WRONLY))
        os.close(os.open(os.fsencode(self.exports) + b"/Strand_\xe4.jpg", os.O_CREAT | os.O_WRONLY))
        r, body = self.request("GET", "/api/dateien")
        self.assertEqual(r.status, 200)
        d = json.loads(body)
        self.assertEqual((d["fotos"], d["exporte_gesamt"]), (1, 1))
        r, body = self.request("GET", "/exporte.zip")
        self.assertEqual(len(zipfile.ZipFile(io.BytesIO(body)).namelist()), 1)

    def test_the_vnc_password_only_for_the_own_page(self):
        self.assertEqual(self.request("GET", "/api/vnc", headers={"X-Shnyp": "1"})[0].status, 503, "before shnyp-web wrote it")
        (self.dir / "vnc-passwort").write_text("Ab+/9xYz")
        self.assertEqual(self.request("GET", "/api/vnc")[0].status, 403)
        r, body = self.request("GET", "/api/vnc", headers={"X-Shnyp": "1"})
        self.assertEqual((r.status, json.loads(body)), (200, {"passwort": "Ab+/9xYz"}))
        # a page still open from the time shnyp was called BR
        self.assertEqual(self.request("GET", "/api/vnc", headers={"X-BR": "1"})[0].status, 200)
        self.assertEqual(self.request("GET", "/api/vnc", headers={"X-BR": "0"})[0].status, 403)
        self.assertEqual(r.getheader("Cache-Control"), "no-store")

    def test_upload_lands_in_the_photo_folder_without_overwriting(self):
        r, body = self.upload("Urlaub am Meer.JPG")
        self.assertEqual(r.status, 201, body)
        self.assertEqual(json.loads(body)["name"], "Urlaub am Meer.JPG")
        r, body = self.upload("Urlaub am Meer.JPG", b"zweites")
        self.assertEqual(json.loads(body)["name"], "Urlaub am Meer (2).JPG")
        self.assertEqual((self.photos / "Urlaub am Meer.JPG").read_bytes(), b"\xff\xd8\xff photo")
        self.assertEqual(sorted(os.listdir(self.photos)), ["Urlaub am Meer (2).JPG", "Urlaub am Meer.JPG"], "no temporary files left")

    def test_upload_names_cannot_leave_the_folder(self):
        for name, saved in [("../../etc/x.jpg", "x.jpg"), ("..\\..\\y.jpg", "y.jpg"), ("/abs/z.nef", "z.nef"), (".hidden.jpg", "hidden.jpg")]:
            r, body = self.upload(name)
            self.assertEqual(r.status, 201, (name, body))
            self.assertEqual(json.loads(body)["name"], saved)
        self.assertEqual(sorted(os.listdir(self.dir)), ["Exporte", "Fotos"])
        self.assertEqual(sorted(os.listdir(self.photos)), ["hidden.jpg", "x.jpg", "y.jpg", "z.nef"])

    def test_videos_music_and_subtitles_are_taken_too(self):
        for name in ["IMG_0001.MOV", "clip.mp4", "song.m4a", "subs.srt"]:
            self.assertEqual(self.upload(name, b"x")[0].status, 201, name)
        self.assertEqual(sorted(os.listdir(self.photos)), ["IMG_0001.MOV", "clip.mp4", "song.m4a", "subs.srt"])

    def test_uploads_that_are_refused(self):
        self.assertEqual(self.upload("a.jpg", headers={})[0].status, 403, "without the page's header")
        self.assertEqual(self.upload("notiz.txt")[0].status, 415)
        self.assertEqual(self.upload("ohne-endung")[0].status, 415)
        self.assertEqual(self.upload("...")[0].status, 400)
        self.assertEqual(self.headers_only("gross.jpg", (1 << 20) + 1).status, 413)
        # mit dem ganzen Inhalt unterwegs: die Absage kommt trotzdem an (kein Abbruch der Verbindung)
        for _ in range(3):
            self.assertEqual(self.upload("gross.jpg", b"x" * (3 << 20))[0].status, 413)
            self.assertEqual(self.upload("notiz.txt", b"x" * (3 << 20))[0].status, 415)
        self.assertEqual(self.request("PUT", "/anderswo/a.jpg", b"x", {"X-Shnyp": "1"})[0].status, 404)
        self.assertEqual(os.listdir(self.photos), [])

    def test_a_broken_upload_leaves_nothing_behind(self):
        c = http.client.HTTPConnection("127.0.0.1", self.server.server_address[1], timeout=10)
        c.putrequest("PUT", "/api/fotos/halb.jpg")
        c.putheader("X-Shnyp", "1")
        c.putheader("Content-Length", "1000")
        c.endheaders()
        c.send(b"x" * 100)
        c.sock.shutdown(1)
        try:
            c.getresponse().read()
        except (http.client.HTTPException, OSError):
            pass
        c.close()
        for _ in range(50):
            if not os.listdir(self.photos):
                break
            threading.Event().wait(0.1)
        self.assertEqual(os.listdir(self.photos), [])

    def test_downloads_only_from_the_export_folder(self):
        (self.exports / "Strand-instagram.jpg").write_bytes(b"export")
        (self.photos / "geheim.jpg").write_bytes(b"original")
        (self.exports / "link.jpg").symlink_to(self.photos / "geheim.jpg")
        r, body = self.request("GET", "/exporte/" + urllib.parse.quote("Strand-instagram.jpg"))
        self.assertEqual((r.status, body), (200, b"export"))
        self.assertIn("attachment", r.getheader("Content-Disposition"))
        for path in ["/exporte/../Fotos/geheim.jpg", "/exporte/%2e%2e/Fotos/geheim.jpg", "/exporte/link.jpg", "/exporte/", "/exporte/nichts.jpg"]:
            self.assertEqual(self.request("GET", path)[0].status, 404, path)

    def test_zip_of_all_exports(self):
        (self.exports / "a.jpg").write_bytes(b"a" * 1000)
        (self.exports / "Tag 2").mkdir()
        (self.exports / "Tag 2" / "ü.jpg").write_bytes(b"b")
        r, body = self.request("GET", "/exporte.zip")
        self.assertEqual(r.status, 200)
        z = zipfile.ZipFile(io.BytesIO(body))
        self.assertEqual(sorted(z.namelist()), ["Tag 2/ü.jpg", "a.jpg"])
        self.assertEqual(z.read("a.jpg"), b"a" * 1000)


class Names(unittest.TestCase):
    def test_safe_name(self):
        self.assertEqual(shnyp.safe_name("a/b\\c.jpg"), "c.jpg")
        self.assertEqual(shnyp.safe_name("..."), "")
        self.assertEqual(shnyp.safe_name("x\x00y\n.jpg"), "xy.jpg")
        long = shnyp.safe_name("ä" * 300 + ".jpg")
        self.assertLessEqual(len(long.encode()), 200)
        self.assertTrue(long.endswith(".jpg"))


class Bridge(Server):
    """Ton und Ablegen: was shnyp-dateien zwischen der Seite und dem shnyp-Fenster weiterreicht."""

    def setUp(self):
        super().setUp()
        h = self.server.RequestHandlerClass
        h.ton_socket = self.dir / "ton.sock"
        h.control_info = self.dir / "control.json"

    def fake_shnyp(self, replies):
        """Ein Steuerkanal wie der von shnyp: prüft das Token, merkt sich die Anfragen."""
        srv = socket.create_server(("127.0.0.1", 0))
        port = srv.getsockname()[1]
        self.server.RequestHandlerClass.control_info.write_text(json.dumps({"port": port, "token": "geheim", "pid": 1}))
        seen = []

        def serve():
            conn, _ = srv.accept()
            with conn, conn.makefile("rwb") as f:
                auth = json.loads(f.readline())
                ok = auth["method"] == "auth" and auth["params"]["token"] == "geheim"
                f.write((json.dumps({"id": auth["id"], "ok": ok}) + "\n").encode())
                f.flush()
                if not ok:
                    return
                req = json.loads(f.readline())
                seen.append(req)
                f.write((json.dumps({"id": req["id"], **replies}) + "\n").encode())
                f.flush()
            srv.close()

        threading.Thread(target=serve, daemon=True).start()
        return seen

    def test_dropped_files_are_placed_where_they_were_let_go(self):
        (self.photos / "clip.mp4").write_bytes(b"v")
        seen = self.fake_shnyp({"ok": True, "result": {"dropped": 1}})
        body = json.dumps({"dateien": ["clip.mp4"], "x": 640.5, "y": 820})
        r, data = self.request("POST", "/api/ablegen", body, {"X-Shnyp": "1", "Content-Type": "application/json"})
        self.assertEqual(r.status, 200, data)
        self.assertEqual(seen[0]["method"], "shnyp.drop")
        self.assertEqual(seen[0]["params"], {"paths": [str(self.photos / "clip.mp4")], "screen": [640.5, 820.0]})

    def test_dropping_needs_the_own_page_an_uploaded_file_and_shnyp(self):
        (self.photos / "a.jpg").write_bytes(b"a")
        body = json.dumps({"dateien": ["a.jpg"]})
        self.assertEqual(self.request("POST", "/api/ablegen", body)[0].status, 403)
        for bad in (["../a.jpg"], ["fehlt.jpg"], [".versteckt"]):
            r, _ = self.request("POST", "/api/ablegen", json.dumps({"dateien": bad}), {"X-Shnyp": "1"})
            self.assertEqual(r.status, 404, bad)
        self.assertEqual(self.request("POST", "/api/ablegen", "kein json", {"X-Shnyp": "1"})[0].status, 400)
        # shnyp without its control channel
        r, data = self.request("POST", "/api/ablegen", body, {"X-Shnyp": "1"})
        self.assertEqual(r.status, 503, data)
        # shnyp refuses (a file it cannot take)
        self.fake_shnyp({"ok": False, "error": "a.jpg: no such file"})
        r, data = self.request("POST", "/api/ablegen", body, {"X-Shnyp": "1"})
        self.assertEqual(r.status, 422)
        self.assertIn("no such file", json.loads(data)["fehler"])

    def test_the_sound_is_passed_on_as_it_comes(self):
        self.assertEqual(self.request("GET", "/api/ton", headers={"X-Shnyp": "1"})[0].status, 503, "shnyp plays nothing")
        sock = self.server.RequestHandlerClass.ton_socket
        srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        srv.bind(str(sock))
        srv.listen()
        pcm = bytes(range(256)) * 64

        def play():
            conn, _ = srv.accept()
            with conn:
                conn.sendall(pcm)

        threading.Thread(target=play, daemon=True).start()
        self.assertEqual(self.request("GET", "/api/ton")[0].status, 403)
        r, data = self.request("GET", "/api/ton", headers={"X-Shnyp": "1"})
        self.assertEqual(r.status, 200)
        self.assertEqual(r.getheader("X-Accel-Buffering"), "no")
        self.assertEqual(data, pcm)
        srv.close()
        r, js = self.request("GET", "/ton.js")
        self.assertEqual(r.status, 200)
        self.assertIn(b'registerProcessor("shnyp-ton"', js)

    def test_the_page_has_the_sound_button(self):
        _, body = self.request("GET", "/")
        self.assertIn(b'id="ton"', body)
        self.assertIn(b"api/ablegen", body)


if __name__ == "__main__":
    unittest.main()
