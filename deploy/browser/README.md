# BR im Browser

BR läuft auf dem Server und wird im Browser bedient: ein virtueller Bildschirm (TigerVNC `Xvnc`)
zeigt das BR-Fenster, noVNC bringt ihn über eine WebSocket-Verbindung auf die Seite. Daneben nimmt
die Seite Fotos, Videos, Musik und Untertitel entgegen und bietet die Exporte zum Herunterladen an. Gerechnet wird auf der CPU
des Servers, eine Grafikkarte ist nicht nötig.

```
Browser ──HTTPS + Login──▶ Reverse-Proxy (Caddy / nginx)
                             ├─ /         ▶ 127.0.0.1:6081  br-dateien  (Seite, Upload, Exporte, Ton)
                             │                 ├─ /api/ton      ◀─ ton.sock ◀─ BR (Wiedergabe)
                             │                 └─ /api/ablegen  ─▶ Steuerkanal ─▶ BR (br.drop)
                             └─ /novnc/*  ▶ 127.0.0.1:6080  websockify  ─▶ Xvnc :7 ─▶ openbox + BR
```

Alles hört nur auf `127.0.0.1`; den VNC-Server selbst erreicht nur websockify (über eine
Socket-Datei im Dienst). Ins Netz kommt BR allein über den Reverse-Proxy, und der verlangt einen
Login. Zusätzlich verlangt der VNC-Server ein Passwort, das bei jedem Start neu ausgewürfelt wird
und das nur die eigene Seite abholen kann: Der Browser schickt den Login auch mit, wenn eine
fremde Webseite eine Verbindung zu BR versucht – ohne das Passwort kommt sie trotzdem nicht weiter.

## Einrichten

Auf einem Debian- (ab 12) oder Ubuntu-Server (ab 22.04) mit mindestens 4 GB RAM (für den Build;
im Betrieb braucht BR rund 1 GB):

```sh
git clone https://github.com/C0mo500/BR.git && cd BR
sudo deploy/browser/install.sh
```

Das Skript installiert die Pakete (TigerVNC, noVNC, openbox, zenity, Mesa), baut BR (Rust wird
bei Bedarf nach `/opt/br/rust` installiert; der erste Build dauert 10–30 Minuten), legt den
Benutzer `br` mit den Ordnern in `/srv/br` an und startet die Dienste `br-web` und `br-dateien`.
Ein zweiter Lauf nach `git pull` aktualisiert BR; Einstellungen und Fotos bleiben.

| Was | Wo |
| --- | --- |
| Programme | `/opt/br/bin` (`br`, `br-web`), `/opt/br/share` |
| Einstellungen | `/etc/br-web.env` (Ports, Ordner, Bildschirm, Renderer; Hinweise in der Datei) |
| Bibliothek | `/srv/br/Bibliothek` |
| Hochgeladene Dateien (die Originale) | `/srv/br/Fotos` |
| Exporte | `/srv/br/Exporte` |
| Dienste | `br-web.service`, `br-dateien.service` |

### Reverse-Proxy mit HTTPS und Login

**Caddy** (holt das Zertifikat selbst): [`Caddyfile.example`](Caddyfile.example) übernehmen, das
Passwort mit `caddy hash-password` erzeugen und eintragen, `systemctl reload caddy`. Eine eigene
Subdomain (`br.example.org`) ist am einfachsten und am sichersten. Ein Pfad einer bestehenden
Domain (`example.org/br/`) geht auch, die Vorlage zeigt beides – aber dann teilt BR den Ursprung
mit allem anderen auf dieser Domain: jede Seite dort (auch eine mit Sicherheitslücke oder mit
hochgeladenem HTML) kann BR bedienen. Den Pfad nur nehmen, wenn die Domain sonst nichts
ausliefert, dem man nicht voll traut.

**nginx**: den Block aus [`nginx.conf.example`](nginx.conf.example) in den `server`-Block mit HTTPS
einfügen, die Login-Datei mit `htpasswd -c /etc/nginx/br.htpasswd NAME` anlegen,
`nginx -t && systemctl reload nginx`.

Wichtig in jedem Proxy (auch Apache, Traefik …):

- `/novnc/` braucht WebSockets (Caddy kann das von selbst, nginx mit den `Upgrade`-Zeilen).
- `/novnc/` für fremde Seiten sperren (403), wie in den Vorlagen: ein fremder `Origin`-Header
  und `Sec-Fetch-Site: cross-site` oder `same-site`.
- Uploads dürfen groß sein und sollen nicht gepuffert werden.
- Unter einem Pfad: `/br` auf `/br/` umleiten.

## Benutzen

1. Die Adresse öffnen und anmelden: oben die Leiste der Seite, darunter BR.
2. **Hochladen** (oder Dateien auf die Seite ziehen): Fotos, Videos, Musik und Untertitel (bis
   8 GB je Datei, einstellbar in `/etc/br-web.env`). Sie landen in `/srv/br/Fotos`.
3. **Video:** in BR auf **Schnitt** „Medien hinzufügen…“ (auf dem Startbildschirm) oder **+ Medien**
   in der Werkzeugleiste. Der Dialog öffnet sich im Upload-Ordner; das erste Video gibt das Format
   vor. **Fotos:** auf der Seite **Foto** über **Importieren**; **Strg+A** wählt alle, **OK**.
   Schon importierte Fotos erkennt BR wieder.
4. Bearbeiten, dann **Exportieren** (oben rechts). Unter **Exporte** auf der Seite liegen die
   Dateien zum Herunterladen, einzeln oder als ZIP. Ein Video erscheint dort erst, wenn es
   vollständig geschrieben ist.

Der Bildschirm passt sich der Größe des Browserfensters an; **⛶** schaltet auf Vollbild.
**Strg+K** öffnet in BR die Befehlssuche (jeder Befehl von BR, FilmCraft und LightCraft, deutsch
oder englisch, mit seiner Taste); **Strg+Alt+1 … 6** wechselt die Seiten. Manche Browser behalten
Strg+K für sich: dann über **⛶** ins Vollbild, dort gehen die Tasten an BR.
Hochgeladenes bleibt auf dem Server: BR bearbeitet es, ohne das Original zu verändern.

### Claude im selben Fenster

`BR_CONTROL=1` (Standard in `/etc/br-web.env`) öffnet BRs Steuerkanal auf `127.0.0.1`. Claude Code
auf dem Server arbeitet dann in dem Fenster, das du im Browser siehst: es schneidet, setzt
Untertitel, exportiert, und du schaust zu.

```sh
claude mcp add br -- sudo -u br env HOME=/srv/br /opt/br/bin/br mcp --connect
```

Den Kanal schützt ein Token, das BR bei jedem Start neu erzeugt und nur für den Benutzer `br`
lesbar ablegt (`/srv/br/.config/BR/control.json`). Ohne das Token wird keine Anfrage ausgeführt.

## Betrieb

```sh
systemctl status br-web br-dateien        # läuft es?
journalctl -u br-web -u br-dateien -f     # Meldungen
systemctl restart br-web br-dateien       # nach Änderungen in /etc/br-web.env
cd BR && git pull && sudo deploy/browser/install.sh   # BR aktualisieren
```

`br-web` startet BR neu, wenn es beendet wird (auch über „Beenden“ im Menü), und den ganzen
Bildschirm, wenn ein Teil ausfällt. Zum Sichern genügt `/srv/br`.

Entfernen: `systemctl disable --now br-web br-dateien`, dann
`/etc/systemd/system/br-{web,dateien}.service`, `/etc/br-web.env` und `/opt/br` löschen;
`/srv/br` und den Benutzer `br` nur, wenn die Fotos weg dürfen.

## Gut zu wissen

- **Eine Person zur Zeit.** Es gibt einen Bildschirm: öffnen zwei Browser die Seite, sehen beide
  dasselbe und bedienen es gemeinsam.
- **Dateien auf das BR-Bild ziehen:** VNC kennt kein Ziehen von Dateien; die Seite fängt sie ab,
  lädt sie hoch und lässt BR sie dort ablegen, wo sie losgelassen wurden (über den Steuerkanal,
  `BR_CONTROL=1`): auf dem Startbildschirm als neues Video, auf der Timeline an der Schnittstelle
  dort, über der Hauptspur als Overlay, auf dem Programm-Monitor an der Stelle im Bild, auf
  **Foto** in die Bibliothek. Auf den Rest der Seite gezogen, landen sie nur im Upload-Ordner.
- **Ton:** **🔇 Ton** oben auf der Seite einschalten (ein Klick ist nötig, Browser spielen sonst
  nichts ab). BR spielt die Wiedergabe dann in die Seite: 16 Bit, 48 kHz, Stereo, rund 0,1 s
  verzögert, wie das Bild über VNC. VNC selbst überträgt keinen Ton. Ohne Ton: `BR_TON=0`.
- **Automatische Untertitel** laden beim ersten Mal ein Sprachmodell (etwa 150 MB) von
  Hugging Face; der Server braucht dafür Internet. Die Erkennung selbst läuft auf dem Server.
- **Tempo:** Vorschau und Export rechnet die CPU. Eine Grafikkarte im Server (Vulkan) nutzt BR
  mit `LIGHTCRAFT_GPU=1`; was dazu noch nötig ist, steht in `/etc/br-web.env`.
- **Ordner** außerhalb von `/srv/br` brauchen eine Freigabe für die Dienste, siehe
  `/etc/br-web.env`.
- **Bildqualität der Übertragung:** Das Bild im Browser ist verlustbehaftet komprimiert. Für die
  Farbbeurteilung zählt der Export.
- **Datei-Dialoge** zeigen das Dateisystem des Servers, mit den Rechten des Benutzers `br`.
- **Ports auf 127.0.0.1** (6080, 6081) haben keinen eigenen Login: wer auf dem Server selbst ein
  Konto hat, erreicht darüber die Seite und BR. Für einen Server, den man allein nutzt, ist das in
  Ordnung.
- **Kleine Fenster:** BR braucht mindestens 1000 × 640 Punkte; ist das Browserfenster kleiner,
  lässt sich die Seite verschieben (oder **⛶** für Vollbild).
