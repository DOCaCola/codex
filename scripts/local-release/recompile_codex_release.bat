@echo off
setlocal

set "SCRIPT_DIR=%~dp0"
for %%I in ("%SCRIPT_DIR%..\..") do set "REPO_DIR=%%~fI"
for %%I in ("%REPO_DIR%\..") do set "WORKSPACE_DIR=%%~fI"
set "CODEX_RS_DIR=%REPO_DIR%\codex-rs"
if not defined CODEX_CARGO_TARGET_DIR set "CODEX_CARGO_TARGET_DIR=C:\temp\codex-target"
rem Use CODEX_BUILD_PROFILE=release for the full production ThinLTO build.
if not defined CODEX_BUILD_PROFILE set "CODEX_BUILD_PROFILE=fast-release"

if not defined CARGO_BIN set "CARGO_BIN=%USERPROFILE%\.cargo\bin\cargo.exe"
if not defined HPATCH_SOURCE_DIR set "HPATCH_SOURCE_DIR=%WORKSPACE_DIR%\hpatch"
if not defined HPATCH_BUILD_OUTPUT set "HPATCH_BUILD_OUTPUT=C:\temp\codex-hpatch-bin\hpatch.exe"

set "DIST_DIR=%WORKSPACE_DIR%\output"

if not exist "%CODEX_RS_DIR%" (
  echo Could not find repo at: %CODEX_RS_DIR%
  exit /b 1
)

if not exist "%CARGO_BIN%" (
  echo Cargo binary not found: %CARGO_BIN%
  exit /b 1
)

if not exist "%HPATCH_SOURCE_DIR%\go.mod" (
  echo hpatch fork not found at: %HPATCH_SOURCE_DIR%
  exit /b 1
)

if not exist "%DIST_DIR%" (
  mkdir "%DIST_DIR%"
  if errorlevel 1 (
    echo Failed to create output directory: %DIST_DIR%
    exit /b 1
  )
)

set "MSYSTEM=UCRT64"
set "CHERE_INVOKING=1"
if /I not "%HPATCH_BUILD_OUTPUT:~0,2%"=="C:" (
  echo HPATCH_BUILD_OUTPUT must be on the C drive: %HPATCH_BUILD_OUTPUT%
  exit /b 1
)
set "HPATCH_BUILD_OUTPUT_BASH=/c/%HPATCH_BUILD_OUTPUT:~3%"
set "HPATCH_BUILD_OUTPUT_BASH=%HPATCH_BUILD_OUTPUT_BASH:\=/%"
"C:\msys64\usr\bin\bash.exe" "%REPO_DIR%\scripts\build-hpatch-companion.sh" --source "%HPATCH_SOURCE_DIR%" --target x86_64-pc-windows-msvc --output "%HPATCH_BUILD_OUTPUT_BASH%"
if errorlevel 1 (
  echo hpatch companion build failed.
  exit /b 1
)
if not exist "%HPATCH_BUILD_OUTPUT%" (
  echo hpatch companion build succeeded but output was not found: %HPATCH_BUILD_OUTPUT%
  exit /b 1
)

pushd "%CODEX_RS_DIR%"
if errorlevel 1 (
  echo Failed to enter codex-rs directory.
  exit /b 1
)

echo Building Codex and package helpers (%CODEX_BUILD_PROFILE% profile)...
set "CARGO_TARGET_DIR=%CODEX_CARGO_TARGET_DIR%"
python "%SCRIPT_DIR%build_codex_release.py" --cargo "%CARGO_BIN%" --profile "%CODEX_BUILD_PROFILE%"
set "BUILD_EXIT=%ERRORLEVEL%"

popd

if not "%BUILD_EXIT%"=="0" (
  echo Build failed with exit code %BUILD_EXIT%.
  exit /b %BUILD_EXIT%
)

echo Assembling complete Codex package...
python "%SCRIPT_DIR%package_codex_release.py" --build-dir "%CODEX_CARGO_TARGET_DIR%\%CODEX_BUILD_PROFILE%" --hpatch-bin "%HPATCH_BUILD_OUTPUT%"
exit /b %ERRORLEVEL%
