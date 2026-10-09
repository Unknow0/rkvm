@echo off

cd /d "%~dp0\.."

echo Building....
cargo build --release --features windows-service
if %errorlevel% neq 0 exit /b %errorlevel%

call windows-service\uninstall.bat
call windows-service\install.bat

pause
