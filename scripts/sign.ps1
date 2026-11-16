# ファイルを 1 つ署名する（10b の設計書 §5.6）。署名の方式は環境変数 FUDE_SIGN_MODE で選ぶ。
#   none（または未設定）: 何もしないで成功する。
#   pfx    : signtool で、FUDE_SIGN_PFX（.pfx のパス）と FUDE_SIGN_PFX_PASSWORD を使って署名する。
#            時刻の印のサーバーは FUDE_SIGN_TIMESTAMP_URL（既定は http://timestamp.digicert.com）。
#   command: FUDE_SIGN_COMMAND の {file} をファイルのパスに置き換えて実行する（署名サービスを呼ぶ手順を入れるため。
#            署名し終わるまで待つ同期のコマンドにする）。
# 使い方: pwsh -NoProfile -File scripts/sign.ps1 <ファイル>
param([Parameter(Mandatory = $true)][string]$File)

$ErrorActionPreference = 'Stop'
$mode = if ($env:FUDE_SIGN_MODE) { $env:FUDE_SIGN_MODE } else { 'none' }

if (-not (Test-Path -LiteralPath $File)) {
    Write-Error "署名するファイルが無い: $File"
    exit 1
}

switch ($mode) {
    'none' {
        exit 0
    }
    'pfx' {
        if (-not $env:FUDE_SIGN_PFX) { Write-Error 'FUDE_SIGN_PFX が無い'; exit 1 }
        $signtool = (Get-Command signtool.exe -ErrorAction SilentlyContinue).Source
        if (-not $signtool) {
            # Windows SDK の一番新しい版の x64 の signtool を使う。
            $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
                Sort-Object { [version]($_.Directory.Parent.Name -replace '[^0-9.]', '') } -Descending |
                Select-Object -First 1 -ExpandProperty FullName
        }
        if (-not $signtool) { Write-Error 'signtool.exe が見つからない（Windows SDK を入れる）'; exit 1 }
        $ts = if ($env:FUDE_SIGN_TIMESTAMP_URL) { $env:FUDE_SIGN_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
        $signArgs = @('sign', '/fd', 'sha256', '/tr', $ts, '/td', 'sha256', '/f', $env:FUDE_SIGN_PFX)
        if ($env:FUDE_SIGN_PFX_PASSWORD) { $signArgs += @('/p', $env:FUDE_SIGN_PFX_PASSWORD) }
        $signArgs += $File
        # パスワードを出さないよう、signtool の出力だけを流す。
        & $signtool @signArgs
        if ($LASTEXITCODE -ne 0) { Write-Error "signtool が失敗した（$LASTEXITCODE）: $File"; exit 1 }
        exit 0
    }
    'command' {
        if (-not $env:FUDE_SIGN_COMMAND) { Write-Error 'FUDE_SIGN_COMMAND が無い'; exit 1 }
        $full = (Resolve-Path -LiteralPath $File).Path
        $cmd = $env:FUDE_SIGN_COMMAND.Replace('{file}', $full)
        & pwsh -NoProfile -Command $cmd
        if ($LASTEXITCODE -ne 0) { Write-Error "署名のコマンドが失敗した（$LASTEXITCODE）: $File"; exit 1 }
        exit 0
    }
    default {
        Write-Error "FUDE_SIGN_MODE が分からない: $mode（none、pfx、command のどれか）"
        exit 1
    }
}
