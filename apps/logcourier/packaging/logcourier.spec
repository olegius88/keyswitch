import sys
from pathlib import Path

root = Path(SPECPATH).parent
extra = root / '.local/qt-deps/unpacked/usr/lib/x86_64-linux-gnu/libxcb-cursor.so.0'
a = Analysis(
    [str(root / 'packaging/launcher.py')], pathex=[str(root / 'src')],
    binaries=[(str(extra), '.')] if extra.exists() and sys.platform != 'win32' else [],
    datas=[], hiddenimports=[], hookspath=[], runtime_hooks=[], excludes=[],
)
pyz = PYZ(a.pure)
gui = EXE(pyz, a.scripts, [], exclude_binaries=True, name='LogCourier',
          console=sys.platform != 'win32')
executables = [gui]
if sys.platform == 'win32':
    cli = EXE(pyz, a.scripts, [], exclude_binaries=True, name='LogCourier-cli', console=True)
    executables.append(cli)
collection = COLLECT(*executables, a.binaries, a.datas, name='LogCourier')
