@echo off
setlocal enabledelayedexpansion
rem Windows wrapper mirroring dwgconv.sh: run the built dwgconv against the
rem local .NET install.  Usage:  dwgconv.cmd <in.(dwg^|dxf)> <out.(dwg^|dxf)>
rem Prefers `dotnet` on PATH; falls back to %DOTNET_ROOT% or %USERPROFILE%\.dotnet.
set "HERE=%~dp0"
set "DLL=%HERE%bin\Release\net10.0\dwgconv.dll"

where dotnet >nul 2>nul
if %ERRORLEVEL%==0 (
    dotnet "%DLL%" %*
    exit /b %ERRORLEVEL%
)

if not defined DOTNET_ROOT set "DOTNET_ROOT=%USERPROFILE%\.dotnet"
"%DOTNET_ROOT%\dotnet.exe" "%DLL%" %*
exit /b %ERRORLEVEL%
