# fude の Windows のインストーラーを作る（10b の設計書 §5.7）。出力は dist/fude-<版>-setup.exe。
# 前もって、WSL の中で scripts/build-linux.sh を実行して target/linux/fude-linux を作っておく（CI では Linux のジョブが渡す）。
# 署名は scripts/sign.ps1（環境変数 FUDE_SIGN_MODE など）。署名しないときは何も設定しなくてよい。
# 使い方: pwsh -NoProfile -File scripts/build-installer.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Test-Path 'target/linux/fude-linux')) {
    Write-Error 'target/linux/fude-linux が無い。WSL の中で sh scripts/build-linux.sh を実行してから、もう一度実行する。'
    exit 1
}

cargo build --release -p fude --locked
if ($LASTEXITCODE -ne 0) { Write-Error 'cargo build が失敗した'; exit 1 }

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
$exe = Join-Path (Resolve-Path $targetDir).Path 'release\fude.exe'
$version = ((cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages | Where-Object name -eq 'fude').version

$sign = Join-Path $root 'scripts\sign.ps1'
& pwsh -NoProfile -File $sign $exe
if ($LASTEXITCODE -ne 0) { Write-Error 'fude.exe の署名が失敗した'; exit 1 }

$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { Write-Error 'ISCC.exe が見つからない（winget install --id JRSoftware.InnoSetup -e で入れる）'; exit 1 }

$isccArgs = @("/DAppVersion=$version", "/DFudeExe=$exe")
$mode = if ($env:FUDE_SIGN_MODE) { $env:FUDE_SIGN_MODE } else { 'none' }
if ($mode -ne 'none') {
    # Inno Setup が、アンインストーラーとインストーラーを sign.ps1 で署名する（$f は署名するファイル、$q は引用符）。
    # 引用符をそのまま書くと、PowerShell が ISCC に \" として渡してしまうので $q を使う。
    $isccArgs += '/DSign'
    $isccArgs += "/Sfudesign=pwsh -NoProfile -File `$q$sign`$q `$f"
}
$isccArgs += 'installer\fude.iss'
& $iscc @isccArgs
if ($LASTEXITCODE -ne 0) { Write-Error 'ISCC が失敗した'; exit 1 }

$setup = Join-Path $root "dist\fude-$version-setup.exe"
Write-Output $setup
