# KeySwitch 0.28.0

## Русский

Одно изменение, и оно про Ubuntu: установленная программа теперь обновляется штатными
средствами системы, без ручной загрузки пакета. Полный перечень — в
[CHANGELOG.md](CHANGELOG.md).

### Файлы выпуска

- `KeySwitch-Setup-0.28.0-x64.exe` — установщик для Windows 10/11 x64. Он не подписан
  сертификатом издателя: SmartScreen покажет предупреждение.
- `KeySwitch-0.28.0-windows-x64.zip` — переносимый архив для Windows.
- `keyswitch_0.28.0_amd64.deb` — Ubuntu/Xubuntu, сеанс X11.
- `KeySwitch-0.28.0-macos-arm64.zip` — Mac на Apple Silicon (M1 и новее), macOS 13 и новее.
- `KeySwitch-0.28.0-macos-x86_64.zip` — Mac на процессоре Intel, macOS 13 и новее.
- `SHA256SUMS` — контрольные суммы всех файлов; сверьте их перед установкой.

Архив для Mac нужен ровно один: сборка для Apple Silicon не запускается на Intel и
наоборот.

### Ubuntu: обновления приходят через APT

Каждая новая версия для Ubuntu раньше означала одно и то же: открыть страницу выпуска,
скачать `.deb`, установить его вручную. Программа умела только показать уведомление о
том, что версия вышла.

Теперь пакет подключает репозиторий обновлений сам, и KeySwitch обновляется вместе с
остальной системой — новую версию предлагают «Обновление приложений» и `sudo apt
upgrade`, наравне с любым другим пакетом. Достаточно один раз поставить
`keyswitch_0.28.0_amd64.deb` с этой страницы; дальше загрузку и установку берёт на себя
APT. Списки пакетов система обновляет по своему расписанию, а `sudo apt update`
показывает новую версию сразу.

Пакет ставит два файла: источник `/etc/apt/sources.list.d/keyswitch.sources` и открытый
ключ `/etc/apt/keyrings/keyswitch-archive-keyring.asc`, которым подписан репозиторий
<https://olegius88.github.io/keyswitch/>. Ключу доверяет только этот источник — в общий
набор доверенных ключей APT он не попадает, и подписывать им что-либо ещё нельзя. Оба
файла принадлежат пакету: строка `Enabled: no` в источнике или его удаление отключают
обновления и переживают установку следующих версий, а `sudo apt purge keyswitch` убирает
оба файла.

Поставить KeySwitch сразу из репозитория, не скачивая пакет вручную, тоже можно —
команды приведены в [README.md](README.md).

Проверка выпусков внутри программы осталась уведомлением: KeySwitch показывает найденную
версию и кнопку «Открыть выпуск». Системный пакет программа в фоне не ставит — это
делает APT с правами, которые для этого и предназначены.

Windows и macOS этот выпуск не меняет.

## English

One change, and it is about Ubuntu: an installed copy now updates through the system's own
tools, with no manual download. The full list is in [CHANGELOG.md](CHANGELOG.md).

### Release files

- `KeySwitch-Setup-0.28.0-x64.exe` — installer for Windows 10/11 x64. It is not signed with
  a publisher certificate, so SmartScreen shows a warning.
- `KeySwitch-0.28.0-windows-x64.zip` — portable archive for Windows.
- `keyswitch_0.28.0_amd64.deb` — Ubuntu/Xubuntu on an X11 session.
- `KeySwitch-0.28.0-macos-arm64.zip` — Mac with Apple silicon (M1 and later), macOS 13 or newer.
- `KeySwitch-0.28.0-macos-x86_64.zip` — Mac with an Intel processor, macOS 13 or newer.
- `SHA256SUMS` — checksums of every file; verify them before installing.

Exactly one Mac archive is the right one: a build for Apple silicon does not run on Intel,
or the other way round.

### Ubuntu: updates arrive through APT

Every new version on Ubuntu used to mean the same steps: open the release page, download
the `.deb`, install it by hand. The application could only tell you that a version had
been published.

The package now registers its update repository itself, and KeySwitch is updated with the
rest of the system: Software Updater and `sudo apt upgrade` offer the new version like any
other package. Install `keyswitch_0.28.0_amd64.deb` from this page once, and APT takes
over the downloading and installing from there. The system refreshes package lists on its
own schedule, and `sudo apt update` shows a new version at once.

The package installs two files: the source
`/etc/apt/sources.list.d/keyswitch.sources` and the public key
`/etc/apt/keyrings/keyswitch-archive-keyring.asc` that signs the repository at
<https://olegius88.github.io/keyswitch/>. The key is trusted for this source alone — it
never joins the set of keys APT trusts everywhere, and it cannot vouch for anything else.
Both files belong to the package: `Enabled: no` in the source, or deleting the source,
stops the updates and survives later versions, while `sudo apt purge keyswitch` removes
both files.

Installing KeySwitch straight from the repository, without downloading a package first, is
possible as well; the commands are in [README.en.md](README.en.md).

Release checking inside the application stays a notification: KeySwitch shows the version
it found and an Open release button. The application never installs the system package in
the background — APT does, with the permissions meant for it.

Windows and macOS are unchanged in this release.

Technical logs may contain evaluated words. Review them before sharing. Private logs and
conversations are excluded from the release.
