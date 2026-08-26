#!/usr/bin/env bash
set -euo pipefail

user_data_home="${XDG_DATA_HOME:-${HOME:?}/.local/share}"
user_config_home="${XDG_CONFIG_HOME:-${HOME:?}/.config}"
application_dir="$user_data_home/keyswitch/app"

rm -f "${HOME:?}/.local/bin/keyswitch"
rm -f "$user_data_home/applications/io.github.olegius88.KeySwitch.desktop"
# Remove the pre-release desktop entry if it was installed by an older checkout.
rm -f "$user_data_home/applications/io.github.keyswitch.KeySwitch.desktop"
rm -f "$user_data_home/icons/hicolor/scalable/apps/keyswitch.svg"
rm -f "$user_data_home/icons/hicolor/scalable/apps/keyswitch-paused.svg"
rm -f "$user_data_home/icons/hicolor/scalable/apps/keyswitch-en.svg"
rm -f "$user_data_home/icons/hicolor/scalable/apps/keyswitch-ru.svg"
rm -f "$user_data_home/icons/hicolor/scalable/apps/keyswitch-flag-us.svg"
rm -f "$user_data_home/icons/hicolor/scalable/apps/keyswitch-flag-ru.svg"
rm -f "$user_config_home/autostart/io.github.olegius88.KeySwitch.desktop"
rm -f "$user_config_home/autostart/io.github.keyswitch.KeySwitch.desktop"
if [[ -d "$application_dir" ]]; then
    rm -rf "$application_dir"
fi

printf 'KeySwitch удалён. Настройки и история сохранены.\n'
printf 'Их расположение: %s и %s\n' "$user_config_home/keyswitch" "$user_data_home/keyswitch"
