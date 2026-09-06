$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
if (-not (Test-Path '.venv\Scripts\python.exe')) {
    py -3 -m venv .venv
    if ($LASTEXITCODE -ne 0) { throw 'Не удалось создать Python venv' }
}
& '.venv\Scripts\python.exe' -c 'import logcourier, PySide6'
if ($LASTEXITCODE -ne 0) {
    & '.venv\Scripts\python.exe' -m pip install -e .
    if ($LASTEXITCODE -ne 0) { throw 'Не удалось установить зависимости' }
}
if ($args.Count -eq 0 -or $args[0] -eq 'gui') {
    Start-Process -FilePath (Join-Path $PSScriptRoot '.venv\Scripts\pythonw.exe') -ArgumentList (@('-m', 'logcourier') + $args)
} else {
    & '.venv\Scripts\python.exe' -m logcourier @args
    exit $LASTEXITCODE
}
