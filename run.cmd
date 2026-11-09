@echo off
setlocal
set "CARGO_HOME=%~dp0target\toolchain\cargo"
set "RUSTUP_HOME=%~dp0target\toolchain\rustup"
set "CARGO_TARGET_DIR=%~dp0target\build"
set "ORT_CACHE_DIR=%~dp0target\toolchain\ort-cache"
set "BILI_MSVC=%~dp0target\toolchain\msvc\VC\Tools\MSVC\14.44.35207"
set "BILI_SDK=%~dp0target\toolchain\sdk\c"
set "BILI_SDK_VERSION=10.0.26100.0"
set "PATH=%CARGO_HOME%\bin;%BILI_MSVC%\bin\Hostx64\x64;%BILI_SDK%\bin\%BILI_SDK_VERSION%\x64;%PATH%"
set "INCLUDE=%BILI_MSVC%\include;%BILI_SDK%\Include\%BILI_SDK_VERSION%\ucrt;%BILI_SDK%\Include\%BILI_SDK_VERSION%\shared;%BILI_SDK%\Include\%BILI_SDK_VERSION%\um;%BILI_SDK%\Include\%BILI_SDK_VERSION%\winrt"
set "LIB=%BILI_MSVC%\lib\x64;%BILI_SDK%\ucrt\x64;%BILI_SDK%\um\x64"
set "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=%BILI_MSVC%\bin\Hostx64\x64\link.exe"
set "CC_x86_64_pc_windows_msvc=cl.exe"
set "AR_x86_64_pc_windows_msvc=lib.exe"
pushd "%~dp0"
cargo run -p frontend --locked %*
set "BILI_RUN_EXIT=%ERRORLEVEL%"
popd
exit /b %BILI_RUN_EXIT%
