@echo off

cd /d "%~dp0\.."

call windows-service\build-install.bat

call windows-service\uninstall.bat
