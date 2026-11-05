#!/usr/bin/env bash
# The real tests of the Linux program: a private X server with the layouts us,ru, a private session bus and a private
# accessibility bus, so that nothing reaches the desktop. With a .deb as the argument, also the packaged program.
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
package="${1:-}"
# The coverage run builds under its own profile; everywhere else these are the release builds.
cargo_profile=(--profile "${KS_E2E_PROFILE:-release}")

# GTK 4 does not use GTK_USE_PORTAL=0 to disable its settings portal, and libadwaita has its own client.
# https://docs.gtk.org/gtk4/running.html#gdk-debug
export GDK_DEBUG="${GDK_DEBUG:+$GDK_DEBUG,}no-portals"
export ADW_DISABLE_PORTAL=1
export GTK_USE_PORTAL=0
export GIO_USE_VFS=local
export GDK_BACKEND=x11
# The tests check input and widgets, not rendering; inside Xvfb the Vulkan renderer once logged a lost device for 25 minutes.
# https://docs.gtk.org/gtk4/running.html#gsk_renderer
export GSK_RENDERER="${GSK_RENDERER:-cairo}"
export KEYSWITCH_RESOURCES="$project_dir/resources"
export KS_E2E=1

in_x_server() {
    # Activated D-Bus services inherit the daemon's DISPLAY, so the daemon starts inside the X server.
    xvfb-run -a -s '-screen 0 1280x800x24 -noreset' dbus-run-session -- bash -c 'setxkbmap -layout us,ru && exec "$@"' -- "$@"
}

cd "$project_dir"
cargo test "${cargo_profile[@]}" -p ks-linux --no-run
GTK_A11Y=none in_x_server cargo test "${cargo_profile[@]}" -p ks-linux --test e2e_x11
GTK_A11Y=none in_x_server cargo test "${cargo_profile[@]}" -p ks-linux --test e2e_gtk
dbus-run-session -- cargo test "${cargo_profile[@]}" -p ks-linux --test e2e_tray

runtime="$(mktemp -d)"
trap 'rm -rf -- "$runtime"' EXIT
XDG_RUNTIME_DIR="$runtime" in_x_server cargo test "${cargo_profile[@]}" -p ks-linux --test e2e_atspi

if [[ -n "$package" ]]; then
    staging="$runtime/package"
    dpkg-deb --extract "$package" "$staging"
    KEYSWITCH_RESOURCES="$staging/usr/lib/keyswitch/resources" KS_E2E_BINARY="$staging/usr/bin/keyswitch" GTK_A11Y=none in_x_server cargo test "${cargo_profile[@]}" -p ks-linux --test e2e_package
fi
