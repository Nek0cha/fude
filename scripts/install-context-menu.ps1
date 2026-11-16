# エクスプローラーの最初の右クリックメニューに「fude で開く」を出す（10f。自分の PC 用）。
# crates/fude-shell-menu の DLL をビルドし、自己署名した sparse package で登録する。配布するインストーラーには入らない。
#
# 使い方:
#   pwsh -NoProfile -File scripts/install-context-menu.ps1                       登録する（作り直しもこれ）
#   pwsh -NoProfile -File scripts/install-context-menu.ps1 -FudeExe <fude.exe>   開く fude.exe を変える（開発用など）
#   pwsh -NoProfile -File scripts/install-context-menu.ps1 -Uninstall            登録を外す
#
# - 既定の fude.exe は、インストーラーの入れ先 %LOCALAPPDATA%\Programs\fude\fude.exe。
# - 初回だけ、自己署名の証明書（CN=fude local）を作り、「信頼されたユーザー」（ローカル コンピューター）に入れる。
#   このとき管理者の許可（UAC）を 1 回求める。証明書は -Uninstall でも消さない。
# - DLL などは %LOCALAPPDATA%\fude\context-menu\<作った時刻>\ に置く。エクスプローラーが DLL をつかんだまま
#   離さないので、作り直すたびに新しいフォルダにし、古いフォルダは消せたら消す。
# - メニューが出ないときは、エクスプローラーを再起動する（Stop-Process -Name explorer）。
# - Windows SDK（makeappx.exe と signtool.exe）が要る。
param(
    [switch]$Uninstall,
    [string]$FudeExe = (Join-Path $env:LOCALAPPDATA 'Programs\fude\fude.exe')
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$packageName = 'Nek0cha.fude.ContextMenu'
$subject = 'CN=fude local'
$base = Join-Path $env:LOCALAPPDATA 'fude\context-menu'

# Appx のコマンドは PowerShell 7 では動かないことがあるので、Windows PowerShell で走らせる。
function Invoke-WinPS([string]$Command) {
    & powershell.exe -NoProfile -NonInteractive -Command $Command
    if ($LASTEXITCODE -ne 0) { throw "Windows PowerShell のコマンドが失敗した（$LASTEXITCODE）: $Command" }
}

function Quote([string]$s) { "'" + $s.Replace("'", "''") + "'" }

function Find-SdkTool([string]$Name) {
    $found = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\$Name" -ErrorAction SilentlyContinue |
        Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
    if (-not $found) { throw "$Name が見つからない（Windows SDK を入れる）" }
    $found.FullName
}

function Remove-Registration {
    Invoke-WinPS "Get-AppxPackage $packageName | Remove-AppxPackage"
}

# 今使っているフォルダ以外を消せるだけ消す。つかまれていて消せないものは次に回す。
function Remove-OldFolders([string]$Keep) {
    if (-not (Test-Path -LiteralPath $base)) { return }
    foreach ($dir in Get-ChildItem -LiteralPath $base -Directory) {
        if ($Keep -and $dir.FullName -eq $Keep) { continue }
        try {
            Remove-Item -LiteralPath $dir.FullName -Recurse -Force -ErrorAction Stop
        } catch {
            Write-Host "使用中で消せなかった（次に消す）: $($dir.FullName)"
        }
    }
}

# コード署名の自己署名の証明書を用意し、拇印を返す。信頼されていなければ管理者の許可を求めて信頼させる。
function Get-FudeCert {
    $cert = Get-ChildItem Cert:\CurrentUser\My |
        Where-Object { $_.Subject -eq $subject -and $_.NotAfter -gt (Get-Date) -and $_.HasPrivateKey } |
        Sort-Object NotAfter -Descending | Select-Object -First 1
    if (-not $cert) {
        Write-Host "自己署名の証明書を作る: $subject"
        $cert = New-SelfSignedCertificate -Type Custom -Subject $subject -KeyUsage DigitalSignature `
            -FriendlyName 'fude local' -CertStoreLocation Cert:\CurrentUser\My `
            -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
    }
    $trusted = Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object { $_.Thumbprint -eq $cert.Thumbprint }
    if (-not $trusted) {
        $cer = Join-Path ([IO.Path]::GetTempPath()) 'fude-local.cer'
        Export-Certificate -Cert $cert -FilePath $cer | Out-Null
        Write-Host '証明書を「信頼されたユーザー」に入れる（管理者の許可を求める）'
        $cmd = "Import-Certificate -FilePath $(Quote $cer) -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null"
        $p = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList @('-NoProfile', '-Command', $cmd)
        Remove-Item -LiteralPath $cer -ErrorAction SilentlyContinue
        if ($p.ExitCode -ne 0) { throw "証明書を信頼させられなかった（$($p.ExitCode)）" }
    }
    $cert.Thumbprint
}

if ($Uninstall) {
    Remove-Registration
    Remove-OldFolders $null
    Write-Host '右クリックメニューを外した'
    exit 0
}

if (-not (Test-Path -LiteralPath $FudeExe -PathType Leaf)) {
    throw "fude.exe が無い: $FudeExe（-FudeExe で指定する）"
}
$FudeExe = (Resolve-Path -LiteralPath $FudeExe).Path

$makeappx = Find-SdkTool 'makeappx.exe'
$signtool = Find-SdkTool 'signtool.exe'

# DLL をビルドする。
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $root 'target-dev' }
& cargo build --release -p fude-shell-menu --manifest-path (Join-Path $root 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw "DLL のビルドが失敗した（$LASTEXITCODE）" }
$dll = Join-Path $env:CARGO_TARGET_DIR 'release\fude_shell_menu.dll'

# 置き場所を組む。
$menuDir = Join-Path $base (Get-Date -Format 'yyyyMMddHHmmss')
New-Item -ItemType Directory -Force -Path $menuDir | Out-Null
$src = Join-Path $root 'installer\context-menu'
Copy-Item -LiteralPath $dll, (Join-Path $src 'AppxManifest.xml'), (Join-Path $src 'logo.png') -Destination $menuDir
[IO.File]::WriteAllText((Join-Path $menuDir 'fude-path.txt'), $FudeExe, (New-Object Text.UTF8Encoding $false))

# パッケージ（マニフェストと logo だけ）を作って署名する。
$work = Join-Path ([IO.Path]::GetTempPath()) 'fude-context-menu-pack'
Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path (Join-Path $work 'content') | Out-Null
Copy-Item -LiteralPath (Join-Path $src 'AppxManifest.xml'), (Join-Path $src 'logo.png') -Destination (Join-Path $work 'content')
$msix = Join-Path $work 'fude-context-menu.msix'
& $makeappx pack /o /nv /d (Join-Path $work 'content') /p $msix
if ($LASTEXITCODE -ne 0) { throw "makeappx が失敗した（$LASTEXITCODE）" }
$thumb = Get-FudeCert
& $signtool sign /fd SHA256 /sha1 $thumb $msix
if ($LASTEXITCODE -ne 0) { throw "signtool が失敗した（$LASTEXITCODE）" }

# 登録し直す。
Remove-Registration
Invoke-WinPS "Add-AppxPackage -Path $(Quote $msix) -ExternalLocation $(Quote $menuDir)"
Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
Remove-OldFolders $menuDir
Write-Host "右クリックメニューを登録した（fude.exe: $FudeExe、置き場所: $menuDir）"
