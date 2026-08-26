#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
user_data_home="${XDG_DATA_HOME:-${HOME:?}/.local/share}"
user_bin_dir="${HOME:?}/.local/bin"
application_dir="$user_data_home/keyswitch/app"
package_dir="$application_dir/keyswitch"
desktop_dir="$user_data_home/applications"
icon_dir="$user_data_home/icons/hicolor/scalable/apps"

python3 - <<'PY'
import ctypes.util
import os
from pathlib import Path
import sys

missing = []
recommended = []
try:
    import gi
    gi.require_version("Gtk", "4.0")
    gi.require_version("Adw", "1")
    from gi.repository import Adw, Gtk  # noqa: F401
except Exception as error:
    missing.append(f"GTK 4 / Libadwaita Python bindings: {error}")
try:
    import dbus  # noqa: F401
except Exception as error:
    missing.append(f"python3-dbus: {error}")
for library in ("X11", "Xtst", "xkbcommon"):
    if not ctypes.util.find_library(library):
        missing.append(f"lib{library}")
if not ctypes.util.find_library("hunspell-1.7") and not ctypes.util.find_library("hunspell"):
    recommended.append("библиотека Hunspell")
data_home = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share"))
dictionary_roots = (
    data_home / "keyswitch" / "dictionaries",
    data_home / "hunspell",
    Path("/usr/share/hunspell"),
    Path("/usr/share/myspell/dicts"),
)
for locale in ("en_US", "ru_RU"):
    found = any(
        (root / f"{locale}.aff").is_file()
        and (root / f"{locale}.dic").is_file()
        for root in dictionary_roots
    )
    if not found:
        recommended.append(f"словарь Hunspell {locale}")
for locale in ("en_US", "ru_RU"):
    if not Path(f"/usr/share/onboard/models/{locale}.lm").is_file():
        recommended.append(f"частотная модель Onboard {locale}")
if missing:
    print("Не хватает системных компонентов:", file=sys.stderr)
    for item in missing:
        print(f"  - {item}", file=sys.stderr)
    print(
        "Установите: sudo apt install python3-gi python3-dbus "
        "gir1.2-gtk-4.0 gir1.2-adw-1 libx11-6 libxtst6 libxkbcommon0 "
        "libhunspell-1.7-0 hunspell-en-us hunspell-ru onboard-data",
        file=sys.stderr,
    )
    raise SystemExit(1)
if recommended:
    print("Предупреждение: качество распознавания будет ниже без:", file=sys.stderr)
    for item in recommended:
        print(f"  - {item}", file=sys.stderr)
    print(
        "Рекомендуется: sudo apt install libhunspell-1.7-0 "
        "hunspell-en-us hunspell-ru onboard-data",
        file=sys.stderr,
    )
PY

# Обновляем пакет поверх существующей пользовательской установки. Это
# намеренно недеструктивно: пользовательские настройки и история находятся
# рядом, а установщик не удаляет ни один каталог.
install -d "$package_dir" "$user_bin_dir" "$desktop_dir" "$icon_dir"
cp -a "$project_dir/src/keyswitch/." "$package_dir/"

launcher="$user_bin_dir/keyswitch"
{
    printf '%s\n' '#!/usr/bin/env bash'
    printf '%s\n' 'set -euo pipefail'
    printf 'export PYTHONPATH=%q\n' "$application_dir"
    printf '%s\n' 'exec python3 -m keyswitch "$@"'
} > "$launcher"
chmod 0755 "$launcher"

desktop_file="$desktop_dir/io.github.olegius88.KeySwitch.desktop"
install -m 0644 "$project_dir/packaging/io.github.olegius88.KeySwitch.desktop" "$desktop_file"
sed -i "s|^Exec=.*|Exec=$launcher|" "$desktop_file"
for icon in "$project_dir"/src/keyswitch/resources/keyswitch*.svg; do
    install -m 0644 "$icon" "$icon_dir/$(basename "$icon")"
done

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$desktop_dir" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$user_data_home/icons/hicolor" >/dev/null 2>&1 || true
fi

printf 'KeySwitch установлен. Запуск: %s\n' "$launcher"
printf 'Также приложение доступно в меню рабочего стола.\n'
