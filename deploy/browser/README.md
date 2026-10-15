# shnyp im Browser

shnyp läuft auf dem Server und wird im Browser bedient: ein virtueller Bildschirm (TigerVNC `Xvnc`)
zeigt das shnyp-Fenster, noVNC bringt ihn über eine WebSocket-Verbindung auf die Seite. Daneben nimmt
die Seite Fotos, Videos, Musik und Untertitel entgegen und bietet die Exporte zum Herunterladen an. Gerechnet wird auf der CPU
des Servers, eine Grafikkarte ist nicht nötig.

```
Browser ──HTTPS + Login──▶ Reverse-Proxy (Caddy / nginx)
                             ├─ /         ▶ 127.0.0.1:6081  shnyp-dateien  (Seite, Upload, Exporte, Ton)
                             │                 ├─ /api/ton      ◀─ ton.sock ◀─ shnyp (Wiedergabe)
                             │                 └─ /api/ablegen  ─▶ Steuerkanal ─▶ shnyp (shnyp.drop)
                             └─ /novnc/*  ▶ 127.0.0.1:6080  websockify  ─▶ Xvnc :7 ─▶ openbox + shnyp
```

Alles hört nur auf `127.0.0.1`; den VNC-Server selbst erreicht nur websockify (über eine
Socket-Datei im Dienst). Ins Netz kommt shnyp allein über den Reverse-Proxy, und der verlangt einen
Login. Zusätzlich verlangt der VNC-Server ein Passwort, das bei jedem Start neu ausgewürfelt wird
und das nur die eigene Seite abholen kann: Der Browser schickt den Login auch mit, wenn eine
fremde Webseite eine Verbindung zu shnyp versucht – ohne das Passwort kommt sie trotzdem nicht weiter.

## Einrichten

Auf einem Debian- (ab 12) oder Ubuntu-Server (ab 22.04) mit mindestens 4 GB RAM (für den Build;
im Betrieb braucht shnyp rund 1 GB):

```sh
git clone https://github.com/C0mo500/shnyp.git && cd shnyp
sudo deploy/browser/install.sh
```

Das Skript installiert die Pakete (TigerVNC, noVNC, openbox, zenity, Mesa), baut shnyp (Rust wird
bei Bedarf nach `/opt/shnyp/rust` installiert; der erste Build dauert 10–30 Minuten), legt den
Benutzer `shnyp` mit den Ordnern in `/srv/shnyp` an und startet die Dienste `shnyp-web` und `shnyp-dateien`.
Ein zweiter Lauf nach `git pull` aktualisiert shnyp; Einstellungen und Fotos bleiben.

**Von BR auf shnyp:** shnyp hieß bis Oktober 2026 BR. Eine Einrichtung von damals (Dienste
`br-web` und `br-dateien`, Benutzer `br`, `/srv/br`, `/etc/br-web.env`) übernimmt das Skript beim
nächsten Lauf: Es hält die alten Dienste an, benennt den Benutzer um, verschiebt `/srv/br` nach
`/srv/shnyp` (`/srv/br` zeigt danach dorthin, damit gespeicherte Pfade gültig bleiben), schreibt die
Werte aus `/etc/br-web.env` in `/etc/shnyp-web.env` und nimmt Ergänzungen aus `systemctl edit` mit.
Bricht ein Lauf ab, bringt ihn der nächste zu Ende. Ist `/srv/br` eine eigene Platte (Einhängepunkt)
oder ein Link woandershin, hält das Skript an, bevor es etwas ändert, und sagt, was zu tun ist.
Der Reverse-Proxy bleibt, wie er ist (dieselben Ports). `/opt/br` und `/etc/br-web.env` werden danach
nicht mehr gebraucht; Claude Code bekommt shnyp neu (`claude mcp remove br`, dann wie unten).

| Was | Wo |
| --- | --- |
| Programme | `/opt/shnyp/bin` (`shnyp`, `shnyp-web`), `/opt/shnyp/share` |
| Einstellungen | `/etc/shnyp-web.env` (Ports, Ordner, Bildschirm, Renderer; Hinweise in der Datei) |
| Bibliothek | `/srv/shnyp/Bibliothek` |
| Hochgeladene Dateien (die Originale) | `/srv/shnyp/Fotos` |
| Exporte | `/srv/shnyp/Exporte` |
| Dienste | `shnyp-web.service`, `shnyp-dateien.service` |

### Reverse-Proxy mit HTTPS und Login

**Caddy** (holt das Zertifikat selbst): [`Caddyfile.example`](Caddyfile.example) übernehmen, das
Passwort mit `caddy hash-password` erzeugen und eintragen, `systemctl reload caddy`. Eine eigene
Subdomain (`shnyp.example.org`) ist am einfachsten und am sichersten. Ein Pfad einer bestehenden
Domain (`example.org/shnyp/`) geht auch, die Vorlage zeigt beides – aber dann teilt shnyp den Ursprung
mit allem anderen auf dieser Domain: jede Seite dort (auch eine mit Sicherheitslücke oder mit
hochgeladenem HTML) kann shnyp bedienen. Den Pfad nur nehmen, wenn die Domain sonst nichts
ausliefert, dem man nicht voll traut.

**nginx**: den Block aus [`nginx.conf.example`](nginx.conf.example) in den `server`-Block mit HTTPS
einfügen, die Login-Datei mit `htpasswd -c /etc/nginx/shnyp.htpasswd NAME` anlegen,
`nginx -t && systemctl reload nginx`.

Wichtig in jedem Proxy (auch Apache, Traefik …):

- `/novnc/` braucht WebSockets (Caddy kann das von selbst, nginx mit den `Upgrade`-Zeilen).
- `/novnc/` für fremde Seiten sperren (403), wie in den Vorlagen: ein fremder `Origin`-Header
  und `Sec-Fetch-Site: cross-site` oder `same-site`.
- Uploads dürfen groß sein und sollen nicht gepuffert werden.
- Unter einem Pfad: `/shnyp` auf `/shnyp/` umleiten.

## Benutzen

1. Die Adresse öffnen und anmelden: oben die Leiste der Seite, darunter shnyp.
2. **Hochladen** (oder Dateien auf die Seite ziehen): Fotos, Videos, Musik und Untertitel (bis
   8 GB je Datei, einstellbar in `/etc/shnyp-web.env`). Sie landen in `/srv/shnyp/Fotos`.
3. **Video:** in shnyp auf **Schnitt** „Medien hinzufügen…“ (auf dem Startbildschirm) oder **+ Medien**
   in der Werkzeugleiste. Der Dialog öffnet sich im Upload-Ordner; das erste Video gibt das Format
   vor. **Fotos:** auf der Seite **Foto** über **Importieren**; **Strg+A** wählt alle, **OK**.
   Schon importierte Fotos erkennt shnyp wieder.
4. Bearbeiten, dann **Exportieren** (oben rechts). Unter **Exporte** auf der Seite liegen die
   Dateien zum Herunterladen, einzeln oder als ZIP. Ein Video erscheint dort erst, wenn es
   vollständig geschrieben ist.

Der Bildschirm passt sich der Größe des Browserfensters an; **⛶** schaltet auf Vollbild.
**Strg+K** öffnet in shnyp die Befehlssuche (jeder Befehl von shnyp, FilmCraft und LightCraft, deutsch
oder englisch, mit seiner Taste); **Strg+Alt+1 … 6** wechselt die Seiten. Manche Browser behalten
Strg+K für sich: dann über **⛶** ins Vollbild, dort gehen die Tasten an shnyp.
Hochgeladenes bleibt auf dem Server: shnyp bearbeitet es, ohne das Original zu verändern.

### Claude im selben Fenster

`SHNYP_CONTROL=1` (Standard in `/etc/shnyp-web.env`) öffnet shnyps Steuerkanal auf `127.0.0.1`. Claude Code
auf dem Server arbeitet dann in dem Fenster, das du im Browser siehst: es schneidet, setzt
Untertitel, exportiert, und du schaust zu.

```sh
claude mcp add shnyp -- sudo -u shnyp env HOME=/srv/shnyp /opt/shnyp/bin/shnyp mcp --connect
```

Den Kanal schützt ein Token, das shnyp bei jedem Start neu erzeugt und nur für den Benutzer `shnyp`
lesbar ablegt (`/srv/shnyp/.config/shnyp/control.json`). Ohne das Token wird keine Anfrage ausgeführt.

## Betrieb

```sh
systemctl status shnyp-web shnyp-dateien        # läuft es?
journalctl -u shnyp-web -u shnyp-dateien -f     # Meldungen
systemctl restart shnyp-web shnyp-dateien       # nach Änderungen in /etc/shnyp-web.env
cd shnyp && git pull && sudo deploy/browser/install.sh   # shnyp aktualisieren
```

`shnyp-web` startet shnyp neu, wenn es beendet wird (auch über „Beenden“ im Menü), und den ganzen
Bildschirm, wenn ein Teil ausfällt. Zum Sichern genügt `/srv/shnyp`.

Entfernen: `systemctl disable --now shnyp-web shnyp-dateien`, dann
`/etc/systemd/system/shnyp-{web,dateien}.service`, `/etc/shnyp-web.env` und `/opt/shnyp` löschen;
`/srv/shnyp` und den Benutzer `shnyp` nur, wenn die Fotos weg dürfen.

## Gut zu wissen

- **Eine Person zur Zeit.** Es gibt einen Bildschirm: öffnen zwei Browser die Seite, sehen beide
  dasselbe und bedienen es gemeinsam.
- **Dateien auf das shnyp-Bild ziehen:** VNC kennt kein Ziehen von Dateien; die Seite fängt sie ab,
  lädt sie hoch und lässt shnyp sie dort ablegen, wo sie losgelassen wurden (über den Steuerkanal,
  `SHNYP_CONTROL=1`): auf dem Startbildschirm als neues Video, auf der Timeline an der Schnittstelle
  dort, über der Hauptspur als Overlay, auf dem Programm-Monitor an der Stelle im Bild, auf
  **Foto** in die Bibliothek. Auf den Rest der Seite gezogen, landen sie nur im Upload-Ordner.
- **Ton:** **🔇 Ton** oben auf der Seite einschalten (ein Klick ist nötig, Browser spielen sonst
  nichts ab). shnyp spielt die Wiedergabe dann in die Seite: 16 Bit, 48 kHz, Stereo, rund 0,1 s
  verzögert, wie das Bild über VNC. VNC selbst überträgt keinen Ton. Ohne Ton: `SHNYP_TON=0`.
- **Automatische Untertitel** laden beim ersten Mal ein Sprachmodell (etwa 150 MB) von
  Hugging Face; der Server braucht dafür Internet. Die Erkennung selbst läuft auf dem Server.
- **Tempo:** Vorschau und Export rechnet die CPU. Eine Grafikkarte im Server (Vulkan) nutzt shnyp
  mit `LIGHTCRAFT_GPU=1`; was dazu noch nötig ist, steht in `/etc/shnyp-web.env`.
- **Ordner** außerhalb von `/srv/shnyp` brauchen eine Freigabe für die Dienste, siehe
  `/etc/shnyp-web.env`.
- **Bildqualität der Übertragung:** Das Bild im Browser ist verlustbehaftet komprimiert. Für die
  Farbbeurteilung zählt der Export.
- **Datei-Dialoge** zeigen das Dateisystem des Servers, mit den Rechten des Benutzers `shnyp`.
- **Ports auf 127.0.0.1** (6080, 6081) haben keinen eigenen Login: wer auf dem Server selbst ein
  Konto hat, erreicht darüber die Seite und shnyp. Für einen Server, den man allein nutzt, ist das in
  Ordnung.
- **Kleine Fenster:** shnyp braucht mindestens 1000 × 640 Punkte; ist das Browserfenster kleiner,
  lässt sich die Seite verschieben (oder **⛶** für Vollbild).
