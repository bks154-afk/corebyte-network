@echo off
title CoreByte (CB) Sovereign Node Miner
color 0A

echo ====================================================================
echo   CoreByte (CB) Layer-1 PoW Miner Quick-Start Launcher
echo ====================================================================
echo.

set /p WALLET="Fill Your Payout Wallet Address (0x...) : "

if "%WALLET%"=="" (
    echo [ERROR] Wallet address can not empty!
    pause
    exit /b
)

echo.
echo [*] Wallet Address Locked: %WALLET%
echo [*] Target Stratum Pool: 32.236.238.232:3333
echo [*] Starting Hardware Hashing Engine...
echo.

if not exist "target\release\miner.exe" (
    echo [*] Pre-compiled binary is not installed, building release binary...
    cargo build --release --bin miner
)

target\release\miner.exe
pause