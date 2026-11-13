@echo off
rem Booth UI in a browser on sample data (no engine, no GPU). Opens on http://localhost:3000
rem SWC refuses a cache under AppData\Local on some PCs; the profile folder works.
if not defined SWC_NATIVE_BINDING_CACHE set "SWC_NATIVE_BINDING_CACHE=%USERPROFILE%\.cache\swc"
if not exist "%SWC_NATIVE_BINDING_CACHE%" mkdir "%SWC_NATIVE_BINDING_CACHE%"
cd /d "%~dp0..\apps\native"
pnpm dev
