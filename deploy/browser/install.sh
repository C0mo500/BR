#!/bin/bash
# shnyp im Browser auf einem Linux-Server einrichten (Debian 12+ / Ubuntu 22.04+), als root:
#
#   sudo deploy/browser/install.sh            # shnyp bauen und alles einrichten
#   sudo deploy/browser/install.sh --bin PFAD # ein fertiges shnyp-Programm nehmen statt zu bauen
#
# Danach laufen die Dienste shnyp-web und shnyp-dateien und hören nur auf 127.0.0.1. Ins Netz kommt shnyp
# erst über einen Reverse-Proxy mit HTTPS und Login: siehe README.md in diesem Ordner.
# Ein zweiter Lauf aktualisiert shnyp und lässt /etc/shnyp-web.env und /srv/shnyp unverändert.
# Eine Einrichtung aus der Zeit, als shnyp noch BR hieß, wird übernommen (Benutzer, Daten, Einstellungen).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
prefix=/opt/shnyp
home=/srv/shnyp
bin=""

while [ $# -gt 0 ]; do
    case "$1" in
        --bin) bin=$(realpath "$2"); shift 2 ;;
        -h | --help) sed -n '2,11p' "$0"; exit 0 ;;
        *) echo "unbekannte Option: $1" >&2; exit 2 ;;
    esac
done

step() { printf '\n== %s\n' "$*"; }

[ "$(id -u)" -eq 0 ] || { echo "Bitte als root ausführen (sudo $0)." >&2; exit 1; }
command -v apt-get >/dev/null || { echo "Dieses Skript kennt nur Debian/Ubuntu (apt-get)." >&2; exit 1; }

# shnyp hieß bis Oktober 2026 BR. Was eine Einrichtung von damals hinterlassen hat: die Dienste
# br-web und br-dateien (mit Ergänzungen aus systemctl edit), den Benutzer br (zu Hause in
# /srv/br), die Daten in /srv/br und die Einstellungen in /etc/br-web.env.
br_user() { id br >/dev/null 2>&1 && [ "$(getent passwd br | cut -d: -f6)" = /srv/br ]; }
br_data() { [ -d /srv/br ] && [ ! -L /srv/br ] && { [ -d /srv/br/Fotos ] || [ -d /srv/br/Bibliothek ] || [ -d /srv/br/Exporte ]; }; }
br_left() {
    local unit
    for unit in /etc/systemd/system/br-web.service /etc/systemd/system/br-dateien.service; do
        if [ -e "$unit" ] || [ -d "$unit.d" ]; then return 0; fi
    done
    br_user || br_data || { [ -e /etc/br-web.env ] && [ ! -e /etc/shnyp-web.env ]; }
}
# vorab, bevor irgendetwas geändert wird: was sich nicht von selbst übernehmen lässt
if br_left; then
    stop() { echo "$*" >&2; exit 1; }
    if mountpoint -q /srv/br; then
        stop "/srv/br ist ein Einhängepunkt. Bitte die Datenplatte stattdessen unter /srv/shnyp einhängen (/etc/fstab) und das Skript noch einmal starten."
    fi
    if [ -L /srv/br ] && [ "$(readlink /srv/br)" != /srv/shnyp ]; then
        stop "/srv/br ist ein Link auf $(readlink /srv/br). Bitte /srv/shnyp dorthin zeigen lassen, /srv/br entfernen und das Skript noch einmal starten."
    fi
    if [ -d /srv/br ] && [ ! -L /srv/br ] && [ -e /srv/shnyp ]; then
        stop "/srv/br und /srv/shnyp gibt es beide. Bitte eines davon wegräumen und das Skript noch einmal starten."
    fi
    if id br >/dev/null 2>&1 && ! br_user && ! id shnyp >/dev/null 2>&1; then
        stop "Den Benutzer br hat nicht die Einrichtung von BR angelegt (zu Hause in $(getent passwd br | cut -d: -f6)). Bitte die Dienste von Hand übernehmen."
    fi
fi

step "Pakete"
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
# libasound2-dev: shnyp spielt den Ton von Videos über ALSA ab; das Paket bringt die Bibliothek zum Bauen
# und zum Laufen mit
apt-get install -y -q --no-install-recommends \
    tigervnc-standalone-server tigervnc-tools novnc python3-websockify openbox zenity python3 \
    libgl1 libglx-mesa0 libgl1-mesa-dri libegl1 libx11-6 libx11-xcb1 libxcursor1 libxi6 libxrandr2 \
    libxkbcommon0 libxkbcommon-x11-0 fonts-dejavu-core \
    ca-certificates curl git build-essential pkg-config \
    libasound2-dev

# das Rust einer Einrichtung von BR (in /opt/br/rust) baut auch shnyp
if [ -d /opt/br/rust ] && [ ! -e "$prefix/rust" ]; then
    install -d "$prefix"
    mv /opt/br/rust "$prefix/rust"
fi

if [ -z "$bin" ]; then
    step "shnyp bauen (beim ersten Mal 10–30 Minuten)"
    rust_ok() {
        command -v cargo >/dev/null && command -v rustc >/dev/null || return 1
        local v
        v=$(rustc --version | awk '{print $2}')
        [ "$(printf '%s\n1.95.0\n' "$v" | sort -V | head -1)" = "1.95.0" ]
    }
    if ! rust_ok || [ -x "$prefix/rust/cargo/bin/cargo" ]; then
        # das eigene Rust in /opt/shnyp/rust: bei jedem Lauf auf den neuesten Stand, damit auch ein
        # späteres shnyp mit höheren Ansprüchen baut
        export RUSTUP_HOME=$prefix/rust/rustup CARGO_HOME=$prefix/rust/cargo PATH=$prefix/rust/cargo/bin:$PATH
        if [ ! -x "$prefix/rust/cargo/bin/cargo" ]; then
            echo "Rust ab 1.95 fehlt: wird nach $prefix/rust installiert (rustup)."
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain stable --no-modify-path
        fi
        rustup update stable
    fi
    (cd "$repo" && cargo build --release --locked -p shnyp)
    bin=$repo/target/release/shnyp
fi
if [ ! -f "$bin" ] || [ ! -x "$bin" ]; then
    echo "kein ausführbares shnyp-Programm: $bin" >&2
    exit 1
fi
"$bin" --version >/dev/null || {
    echo "$bin startet auf diesem System nicht (mit --bin: auf derselben oder einer älteren Distribution bauen)." >&2
    exit 1
}

# Die Einrichtung von BR übernehmen, sobald das neue Programm bereitsteht. Jeder Schritt hängt an dem,
# was von BR noch da ist: ein abgebrochener Lauf geht beim nächsten zu Ende.
if br_left; then
    step "Einrichtung von BR übernehmen"
    systemctl disable --now br-web.service br-dateien.service 2>/dev/null || true
    for unit in web dateien; do
        # eigene Ergänzungen (systemctl edit) gehen mit, mit den neuen Namen
        old=/etc/systemd/system/br-$unit.service.d new=/etc/systemd/system/shnyp-$unit.service.d
        if [ -d "$old" ] && [ ! -e "$new" ]; then
            mv "$old" "$new"
            find "$new" -name '*.conf' -exec sed -i \
                -e 's#/opt/br/bin/br-web\([[:space:]"]\|$\)#/opt/shnyp/bin/shnyp-web\1#g' \
                -e 's#/opt/br/bin/br\([[:space:]"]\|$\)#/opt/shnyp/bin/shnyp\1#g' \
                -e 's#/\(srv\|opt\)/br\([/[:space:]"]\|$\)#/\1/shnyp\2#g' \
                -e 's/\bBR_/SHNYP_/g' {} +
        fi
        rm -f "/etc/systemd/system/br-$unit.service"
    done
    systemctl daemon-reload
    if br_data; then
        mv /srv/br "$home"
        # Pfade, die Bibliothek und Videoprojekte gespeichert haben (/srv/br/Fotos/…), bleiben gültig
        ln -s "$home" /srv/br
        echo "/srv/br ist jetzt $home (/srv/br zeigt dorthin)."
    fi
    if [ -e /etc/br-web.env ] && [ ! -e /etc/shnyp-web.env ]; then
        # die neue Vorlage mit den Werten der alten Datei, unter den neuen Namen
        python3 - "$here/shnyp-web.env" /etc/br-web.env /etc/shnyp-web.env.neu <<'PY'
import re, sys
template, old, out = sys.argv[1:]
name = re.compile(r"\s*([A-Za-z_][A-Za-z0-9_]*)=")
def renamed(line):
    line = re.sub(r"^(\s*)BR_", r"\1SHNYP_", line.strip())
    line = re.sub(r"/opt/br/bin/br(-web)?(?=[\s\"']|$)", lambda m: "/opt/shnyp/bin/shnyp" + (m.group(1) or ""), line)
    return re.sub(r"/(srv|opt)/br(?=[/\s\"':]|$)", r"/\1/shnyp", line)
values = {}
for line in open(old, encoding="utf-8"):
    if name.match(line) and not line.lstrip().startswith("#"):
        line = renamed(line)
        values[name.match(line).group(1)] = line
lines = []
for line in open(template, encoding="utf-8"):
    m = name.match(line)
    lines.append(values.pop(m.group(1)) + "\n" if m and not line.startswith(" ") and m.group(1) in values else line)
if values:
    lines.append("\n# aus /etc/br-web.env\n")
    lines.extend(v + "\n" for v in values.values())
open(out, "w", encoding="utf-8").write("".join(lines))
PY
        chmod --reference=/etc/br-web.env /etc/shnyp-web.env.neu
        mv /etc/shnyp-web.env.neu /etc/shnyp-web.env
        echo "Die Einstellungen aus /etc/br-web.env stehen jetzt in /etc/shnyp-web.env."
    fi
    if br_user && ! id shnyp >/dev/null 2>&1; then
        # was als br noch läuft (etwa ein `br mcp --connect`), hält den Benutzer fest
        if pkill -u br; then sleep 2; fi
        usermod -l shnyp -d "$home" br
        if getent group br >/dev/null && ! getent group shnyp >/dev/null; then
            groupmod -n shnyp br
        fi
    fi
    migrated=1
fi

step "Benutzer shnyp und Ordner in $home"
if ! id shnyp >/dev/null 2>&1; then
    useradd --system --home-dir "$home" --create-home --shell /usr/sbin/nologin shnyp
fi
install -d -o shnyp -g shnyp -m 750 "$home"
install -d -o shnyp -g shnyp -m 755 "$home/Bibliothek" "$home/Fotos" "$home/Exporte"

step "Programme nach $prefix"
install -d "$prefix/bin" "$prefix/share"
install -m 755 "$bin" "$prefix/bin/shnyp.new"
mv -f "$prefix/bin/shnyp.new" "$prefix/bin/shnyp"
install -m 755 "$here/shnyp-web" "$prefix/bin/shnyp-web"
install -m 644 "$here/shnyp-dateien.py" "$here/openbox-rc.xml" "$prefix/share/"
if [ -e /etc/shnyp-web.env ]; then
    echo "/etc/shnyp-web.env bleibt wie es ist (Vorlage: $here/shnyp-web.env)."
else
    install -m 644 "$here/shnyp-web.env" /etc/shnyp-web.env
fi
install -m 644 "$here/shnyp-web.service" "$here/shnyp-dateien.service" /etc/systemd/system/

step "Dienste starten"
systemctl daemon-reload
systemctl enable shnyp-web.service shnyp-dateien.service >/dev/null
systemctl restart shnyp-web.service shnyp-dateien.service

# shellcheck disable=SC1091
. /etc/shnyp-web.env
check() {
    python3 - "$1" <<'EOF'
import sys, time, urllib.request
for _ in range(60):
    try:
        # ohne einen Proxy aus http_proxy: es geht um 127.0.0.1
        urllib.request.build_opener(urllib.request.ProxyHandler({})).open(sys.argv[1], timeout=2).read(64)
        sys.exit(0)
    except Exception:
        time.sleep(0.5)
sys.exit(1)
EOF
}
ok=1
if check "http://127.0.0.1:${SHNYP_FILES_PORT:-6081}/"; then
    echo "shnyp-dateien läuft (127.0.0.1:${SHNYP_FILES_PORT:-6081})"
else
    echo "shnyp-dateien antwortet nicht: journalctl -u shnyp-dateien" >&2
    ok=0
fi
if check "http://127.0.0.1:${SHNYP_NOVNC_PORT:-6080}/vnc.html"; then
    echo "noVNC läuft (127.0.0.1:${SHNYP_NOVNC_PORT:-6080})"
else
    echo "noVNC antwortet nicht: journalctl -u shnyp-web" >&2
    ok=0
fi
[ "$ok" = 1 ] || exit 1

if [ "${migrated:-0}" = 1 ]; then
    cat <<EOF

Von BR übernommen. Der Reverse-Proxy bleibt, wie er ist (dieselben Ports). /opt/br und
/etc/br-web.env werden nicht mehr gebraucht. Claude Code auf dem Server bekommt shnyp neu:
  claude mcp remove br
  claude mcp add shnyp -- sudo -u shnyp env HOME=$home $prefix/bin/shnyp mcp --connect
EOF
    exit 0
fi

cat <<EOF

Fertig. Jetzt den Reverse-Proxy einrichten (HTTPS + Login), siehe $here/README.md:
  /         -> 127.0.0.1:${SHNYP_FILES_PORT:-6081}   (Seite, Upload, Exporte)
  /novnc/*  -> 127.0.0.1:${SHNYP_NOVNC_PORT:-6080}   (Präfix /novnc entfernen; WebSocket; fremde
                                     Origin-Header und Sec-Fetch-Site cross-site/same-site
                                     mit 403 abweisen – siehe README, "Wichtig in jedem Proxy")
EOF
