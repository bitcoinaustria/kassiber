@echo off
setlocal
set "KASSIBER_SIDECAR=%~dp0..\binaries\kassiber-cli-x86_64-pc-windows-msvc.exe"
if not exist "%KASSIBER_SIDECAR%" (
  echo Kassiber CLI sidecar not found: "%KASSIBER_SIDECAR%" 1>&2
  exit /b 127
)
"%KASSIBER_SIDECAR%" %*
exit /b %ERRORLEVEL%
