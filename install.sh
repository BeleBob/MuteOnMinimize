#!/usr/bin/env bash
# Per-user install for development: builds the service, installs everything
# under ~/.local and ~/.config, enables the KWin script. No root needed.
#
#   ./install.sh               build, install or update, enable
#   ./install.sh --uninstall   disable and remove

set -euo pipefail
cd "$(dirname "$0")"

data_home=${XDG_DATA_HOME:-$HOME/.local/share}
config_home=${XDG_CONFIG_HOME:-$HOME/.config}
make_vars=(
    PREFIX="$HOME/.local"
    DATADIR="$data_home"
    SYSTEMDUSERUNITDIR="$config_home/systemd/user"
)
script_id=muteonminimize
unit=muteonminimized.service

reload_buses() {
    systemctl --user daemon-reload
    busctl --user call org.freedesktop.DBus /org/freedesktop/DBus \
        org.freedesktop.DBus ReloadConfig >/dev/null
}

set_script_enabled() {
    kwriteconfig6 --file kwinrc --group Plugins --key "${script_id}Enabled" "$1"
    # Unload first so an updated script is read again.
    qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript "$script_id" >/dev/null || true
    qdbus6 org.kde.KWin /KWin reconfigure
}

if [[ ${1:-} == --uninstall ]]; then
    set_script_enabled false
    kwriteconfig6 --file kwinrc --group Plugins --key "${script_id}Enabled" --delete
    systemctl --user stop "$unit" 2>/dev/null || true
    make uninstall "${make_vars[@]}"
    reload_buses
    echo "Removed. Settings in ~/.config/kwinrc [Script-$script_id] were kept."
    exit 0
fi

make build
make install "${make_vars[@]}"
reload_buses
# A running service keeps the old binary until restarted.
systemctl --user try-restart "$unit"
set_script_enabled true

echo "Installed. Choose applications in System Settings → Window Management → KWin Scripts → Mute on Minimize."
