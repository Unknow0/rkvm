@echo off

set SERVICE_NAME=RkvmService
set BASE_PATH=C:\ProgramData\rkvm
set SERVICE_PATH=%BASE_PATH%\rkvm-service.exe

cd /d "%~dp0\.."

call windows-service\uninstall.bat

echo Building....
cargo build --release
if %errorlevel% neq 0 exit /b %errorlevel%

call windows-service\install.bat

timeout /t 5

call windows-service\uninstall.bat
