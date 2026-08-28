# KeySwitch 0.5.0

## Русский

В этом выпуске KeySwitch получил безопасную проверку и установку обновлений.

- KeySwitch проверяет стабильные GitHub Releases через 30 секунд после запуска
  и затем каждые шесть часов. Автоматическую проверку можно отключить.
- В Windows новый Setup EXE скачивается и устанавливается автоматически, если
  включена соответствующая опция. После тихой установки KeySwitch запускается
  снова в фоне.
- Перед запуском установщика KeySwitch проверяет имя и размер файла, HTTPS-адрес
  загрузки и опубликованный GitHub SHA-256 digest. Повреждённый или неполный
  файл не запускается.
- В Ubuntu приложение уведомляет о новой версии и открывает страницу выпуска;
  установка DEB остаётся под явным контролем пользователя и APT.
- В настройках Windows и Ubuntu появилась отдельная страница «Обновления» с
  текущим статусом, ручной проверкой и ссылкой на выпуск.

Пакеты выпуска проходят строгую проверку типов, 100% покрытия строк и ветвей,
реальные X11/Win32 E2E, проверку нативного DEB, тихую установку и сценарий
автоматического перезапуска Windows Setup.

### Установка

- Windows 10/11 x64: `KeySwitch-Setup-0.5.0-x64.exe` или переносимый
  `KeySwitch-0.5.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.5.0_amd64.deb`.

Windows Setup пока не подписан сертификатом издателя. Нативная Wayland-сессия
Linux пока не поддерживается.

## English

This release adds secure update checks and installation to KeySwitch.

- KeySwitch checks stable GitHub Releases 30 seconds after startup and every
  six hours thereafter. Automatic checks can be disabled.
- On Windows, a new Setup EXE is downloaded and installed automatically when
  the corresponding option is enabled. KeySwitch relaunches in the background
  after the silent installation.
- Before starting the installer, KeySwitch validates the filename, byte count,
  HTTPS download origin and GitHub-published SHA-256 digest. Corrupt or partial
  files are rejected.
- On Ubuntu, KeySwitch notifies about a new release and opens its release page;
  DEB replacement remains an explicit user-authorized APT operation.
- Windows and Ubuntu settings now contain a dedicated Updates page with live
  status, a manual check and a release link.

Release packages pass strict type checking, 100% line and branch coverage,
real X11/Win32 E2E tests, native DEB verification, silent Windows Setup
installation and automatic relaunch testing.

### Installation

- Windows 10/11 x64: `KeySwitch-Setup-0.5.0-x64.exe` or the portable
  `KeySwitch-0.5.0-windows-x64.zip`.
- Ubuntu 26.04 x64/X11: `sudo apt install ./keyswitch_0.5.0_amd64.deb`.

The Windows installer is not yet signed with a publisher certificate. Native
Linux Wayland sessions are not supported yet.
