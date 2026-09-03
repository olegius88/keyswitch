# KeySwitch 0.10.0

## Русский

Выпуск про журнал и про сам выпуск: технический журнал теперь ротируется по
режиму, в котором он ведётся, папку журнала можно открыть кнопкой, а релиз
целиком выполняется одной командой. Сертифицированная модель намерения
`intent-v1-6ece07f881ec` не изменилась.

- Журнал ротируется по режиму. Обычная работа хранит 1 МБ в 3 файлах, а режим
  диагностики, который записывает строку на каждое разобранное слово, хранит
  5 МБ в 6 файлах. Включение режима диагностики начинает новый файл, поэтому
  журнал, приложенный к отчёту о проблеме, содержит только нужный сеанс;
  выключение лишь сужает бюджет и сохраняет уже записанное. Переключатель в
  настройках действует сразу, без перезапуска приложения.
- На странице диагностики появилась кнопка, открывающая папку журнала: в
  файловом менеджере в Linux и в проводнике в Windows. Рядом указан текущий
  бюджет ротации.
- Настройка журнала стала общей для обеих платформ (`keyswitch.logsetup`)
  вместо двух почти одинаковых копий в Linux- и Windows-приложении.
- `tools/release.py` выполняет выпуск одной командой: переносит версию во все
  файлы, где она указана, закрывает раздел `Unreleased` в `CHANGELOG.md`,
  проверяет `RELEASE_NOTES.md`, запускает полный контур проверки, коммитит,
  ставит тег, пушит и ждёт, пока сборка опубликует DEB, Windows Setup EXE, ZIP
  и `SHA256SUMS`. Любой невыполненный шаг останавливает работу с текстом
  ошибки, а повторный запуск после сбоя продолжает с места остановки.

Локальный контур выпуска: строгий `mypy`, автоматические тесты со 100%
покрытием строк и ветвей, единый отсоединённый прогон `tools/release_pipeline.py`.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.10.0-x64.exe` или переносимый
  `KeySwitch-0.10.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.10.0_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

A release about the log and about releasing: the technical log now rotates by
the mode it is kept in, a button opens the folder that holds it, and a release
is performed by one command. The certified intent model
`intent-v1-6ece07f881ec` is unchanged.

- The log rotates by mode. Ordinary operation keeps 1 MB in 3 files, while the
  diagnostics mode, which writes a line for every evaluated word, keeps 5 MB in
  6 files. Turning the diagnostics mode on starts a fresh file, so the log
  attached to a problem report contains that session only; turning it off only
  shrinks the budget and keeps what was recorded. The switch in the settings
  takes effect at once, without restarting the application.
- The diagnostics page has a button that opens the folder holding the log, in
  the file manager on Linux and in Explorer on Windows. The current rotation
  budget is stated next to it.
- Log configuration is now shared by both platforms (`keyswitch.logsetup`)
  instead of two nearly identical copies in the Linux and Windows applications.
- `tools/release.py` performs a release in one command: it propagates the
  version to every file that spells it, closes the `Unreleased` section of
  `CHANGELOG.md`, checks `RELEASE_NOTES.md`, runs the complete verification
  contour, commits, tags, pushes and waits until the build publishes the DEB,
  the Windows Setup EXE, the ZIP and `SHA256SUMS`. Any step that does not hold
  stops the run with a message, and a re-run after a failure continues where it
  stopped.

The local release gate passes strict `mypy`, automated tests with mandatory
100% line and branch coverage, and one detached `tools/release_pipeline.py`
run.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.10.0-x64.exe` or the portable
  `KeySwitch-0.10.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.10.0_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.
