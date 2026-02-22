@echo off

set SERVICE_NAME=RkvmService
set BASE_PATH=C:\ProgramData\rkvm
set SERVICE_PATH=%BASE_PATH%\rkvm-service.exe

cd /d "%~dp0\.."

echo Copying release
if not exist "%BASE_PATH%" mkdir "%BASE_PATH%"
del "%BASE_PATH%"\*.log "%BASE_PATH%"\*.exe
copy target\release\rkvm-service.exe "%BASE_PATH%"
copy target\release\rkvm-client.exe "%BASE_PATH%"

echo Installing service...
sc create "%SERVICE_NAME%" binPath= "%SERVICE_PATH%" start= auto  obj= LocalSystem
sc failure "%SERVICE_NAME%" reset= 86400 actions= restart/1000/restart/1500/restart/10000
sc failureflag "%SERVICE_NAME%" 1

echo Starting service...
sc start "%SERVICE_NAME%"

echo Done.

