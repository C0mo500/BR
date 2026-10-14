#!/bin/bash
# BR im Browser auf einem Linux-Server einrichten (Debian 12+ / Ubuntu 22.04+), als root:
#
#   sudo deploy/browser/install.sh            # BR bauen und alles einrichten
#   sudo deploy/browser/install.sh --bin PFAD # ein fertiges br-Programm nehmen statt zu bauen
#
# Danach laufen die Dienste br-web und br-dateien und hören nur auf 127.0.0.1. Ins Netz kommt BR
# erst über einen Reverse-Proxy mit HTTPS und Login: siehe README.md in diesem Ordner.
# Ein zweiter Lauf aktualisiert BR und lässt /etc/br-web.env und /srv/br unverändert.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
prefix=/opt/br
home=/srv/br
bin=""

while [ $# -gt 0 ]; do
    case "$1" in
        --bin) bin=$(realpath "$2"); shift 2 ;;
        -h | --help) sed -n '2,10p' "$0"; exit 0 ;;
        *) echo "unbekannte Option: $1" >&2; exit 2 ;;
    esac
done

step() { printf '\n== %s\n' "$*"; }

[ "$(id -u)" -eq 0 ] || { echo "Bitte als root ausführen (sudo $0)." >&2; exit 1; }
command -v apt-get >/dev/null || { echo "Dieses Skript kennt nur Debian/Ubuntu (apt-get)." >&2; exit 1; }

step "Pakete"
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get install -y -q --no-install-recommends \
    tigervnc-standalone-server tigervnc-tools novnc python3-websockify openbox zenity python3 \
    libgl1 libglx-mesa0 libgl1-mesa-dri libegl1 libx11-6 libx11-xcb1 libxcursor1 libxi6 libxrandr2 \
    libxkbcommon0 libxkbcommon-x11-0 fonts-dejavu-core \
    ca-certificates curl git build-essential pkg-config

if [ -z "$bin" ]; then
    step "BR bauen (beim ersten Mal 10–30 Minuten)"
    rust_ok() {
        command -v cargo >/dev/null && command -v rustc >/dev/null || return 1
        local v
        v=$(rustc --version | awk '{print $2}')
        [ "$(printf '%s\n1.95.0\n' "$v" | sort -V | head -1)" = "1.95.0" ]
    }
    if ! rust_ok || [ -x "$prefix/rust/cargo/bin/cargo" ]; then
        # das eigene Rust in /opt/br/rust: bei jedem Lauf auf den neuesten Stand, damit auch ein
        # späteres BR mit höheren Ansprüchen baut
        export RUSTUP_HOME=$prefix/rust/rustup CARGO_HOME=$prefix/rust/cargo PATH=$prefix/rust/cargo/bin:$PATH
        if [ ! -x "$prefix/rust/cargo/bin/cargo" ]; then
            echo "Rust ab 1.95 fehlt: wird nach $prefix/rust installiert (rustup)."
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain stable --no-modify-path
        fi
        rustup update stable
    fi
    (cd "$repo" && cargo build --release --locked -p br)
    bin=$repo/target/release/br
fi
if [ ! -f "$bin" ] || [ ! -x "$bin" ]; then
    echo "kein ausführbares br-Programm: $bin" >&2
    exit 1
fi
"$bin" --version >/dev/null || {
    echo "$bin startet auf diesem System nicht (mit --bin: auf derselben oder einer älteren Distribution bauen)." >&2
    exit 1
}

step "Benutzer br und Ordner in $home"
if ! id br >/dev/null 2>&1; then
    useradd --system --home-dir "$home" --create-home --shell /usr/sbin/nologin br
fi
install -d -o br -g br -m 750 "$home"
install -d -o br -g br -m 755 "$home/Bibliothek" "$home/Fotos" "$home/Exporte"

step "Programme nach $prefix"
install -d "$prefix/bin" "$prefix/share"
install -m 755 "$bin" "$prefix/bin/br.new"
mv -f "$prefix/bin/br.new" "$prefix/bin/br"
install -m 755 "$here/br-web" "$prefix/bin/br-web"
install -m 644 "$here/br-dateien.py" "$here/openbox-rc.xml" "$prefix/share/"
if [ -e /etc/br-web.env ]; then
    echo "/etc/br-web.env bleibt wie es ist (Vorlage: $here/br-web.env)."
else
    install -m 644 "$here/br-web.env" /etc/br-web.env
fi
install -m 644 "$here/br-web.service" "$here/br-dateien.service" /etc/systemd/system/

step "Dienste starten"
systemctl daemon-reload
systemctl enable br-web.service br-dateien.service >/dev/null
systemctl restart br-web.service br-dateien.service

# shellcheck disable=SC1091
. /etc/br-web.env
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
if check "http://127.0.0.1:${BR_FILES_PORT:-6081}/"; then
    echo "br-dateien läuft (127.0.0.1:${BR_FILES_PORT:-6081})"
else
    echo "br-dateien antwortet nicht: journalctl -u br-dateien" >&2
    ok=0
fi
if check "http://127.0.0.1:${BR_NOVNC_PORT:-6080}/vnc.html"; then
    echo "noVNC läuft (127.0.0.1:${BR_NOVNC_PORT:-6080})"
else
    echo "noVNC antwortet nicht: journalctl -u br-web" >&2
    ok=0
fi
[ "$ok" = 1 ] || exit 1

cat <<EOF

Fertig. Jetzt den Reverse-Proxy einrichten (HTTPS + Login), siehe $here/README.md:
  /         -> 127.0.0.1:${BR_FILES_PORT:-6081}   (Seite, Upload, Exporte)
  /novnc/*  -> 127.0.0.1:${BR_NOVNC_PORT:-6080}   (Präfix /novnc entfernen; WebSocket;
                                     fremde Origin-Header abweisen)
EOF
