param(
    [ValidateSet("x64", "arm64")] [string]$Arch = "x64",
    [string]$QtDir = $env:QT_DIR
)
$ErrorActionPreference = "Stop"
$Root = Resolve-Path "$PSScriptRoot\..\.."
$Here = "$Root\packaging\windows"
$Work = "$Root\build\package-windows"
$Stage = "$Work\stage"
$Out = if ($env:OUT) { $env:OUT } else { "$Root\dist" }
$Version = (Select-String -Path "$Root\CMakeLists.txt" -Pattern 'project\(Setu VERSION ([0-9.]+)').Matches[0].Groups[1].Value
if (-not $QtDir) { throw "Set QT_DIR or pass -QtDir, for example C:\Qt\6.8.3\msvc2022_64" }
$Target = if ($Arch -eq "arm64") { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" }
$env:SETU_PACKAGING = "1"
$env:CARGO_TARGET_DIR = "$Work\cargo"
if (-not $env:SETU_BUILD_ID) { $env:SETU_BUILD_ID = (git -C $Root rev-parse --short HEAD) 2>$null }
if (-not $env:SHERPA_LIB_ARCHIVE) { throw "Set SHERPA_LIB_ARCHIVE and SHERPA_LIB_SHA256 for the Windows sherpa-onnx shared build" }

function Step($t) { Write-Host "`n== $t" }
function Run($exe, [string[]]$a) { & $exe @a; if ($LASTEXITCODE -ne 0) { throw "$exe failed with $LASTEXITCODE" } }

Step "Dependencies"
Run "bash" @("$Root/scripts/fetch-deps.sh")
Run "cmake" @("-S", "$Root\llama.cpp", "-B", "$Work\llama", "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release", "-DBUILD_SHARED_LIBS=ON",
    "-DGGML_BACKEND_DL=ON", "-DGGML_CPU_ALL_VARIANTS=$(if ($Arch -eq 'x64') { 'ON' } else { 'OFF' })", "-DGGML_NATIVE=OFF",
    "-DLLAMA_BUILD_TESTS=OFF", "-DLLAMA_BUILD_EXAMPLES=OFF", "-DLLAMA_BUILD_TOOLS=OFF", "-DLLAMA_BUILD_SERVER=OFF", "-DLLAMA_BUILD_COMMON=OFF")
Run "cmake" @("--build", "$Work\llama", "--target", "llama")

Step "Daemon and engines"
$env:LLAMA_CPP_LIB = "$Work\llama\bin"
Push-Location "$Root\daemon"
Run "cargo" @("build", "--release", "--locked", "--target", $Target, "--features", "setu-audio/portable", "-p", "setu-daemon", "-p", "setu-engine", "-p", "setu-system", "-p", "setu-facecheck", "-p", "setu-credprov")
Pop-Location

Step "Interface"
Run "cmake" @("-S", "$Root", "-B", "$Work\qt", "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_PREFIX_PATH=$QtDir", "-DSETU_BUILD_TESTS=OFF", "-DSETU_BUILD_ID=$env:SETU_BUILD_ID")
Run "cmake" @("--build", "$Work\qt")

Step "Staging"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Force "$Stage\share\models" | Out-Null
Copy-Item "$Work\qt\setu-shell.exe", "$Work\qt\setu-ui.exe" $Stage
Copy-Item "$env:CARGO_TARGET_DIR\$Target\release\setu-daemon.exe", "$env:CARGO_TARGET_DIR\$Target\release\setu-engine.exe", "$env:CARGO_TARGET_DIR\$Target\release\setu-system.exe", "$env:CARGO_TARGET_DIR\$Target\release\setu-facecheck.exe", "$env:CARGO_TARGET_DIR\$Target\release\setu_credprov.dll" $Stage
Copy-Item "$Root\third_party\sherpa-onnx\lib\*.dll" $Stage
Copy-Item "$Work\llama\bin\*.dll" $Stage
Copy-Item -Recurse "$Root\third_party\bundled\models\*" "$Stage\share\models"
Get-ChildItem -Recurse -Force "$Stage\share\models" -Filter ".stamp" | Remove-Item -Force
Copy-Item "$Root\third_party\bundled\benchmark.wav" "$Stage\share\"
Copy-Item "$Root\assets\licenses\setu.txt" "$Stage\LICENSE.txt"
Copy-Item "$Work\qt\generated\licenses\third_party.json" "$Stage\third-party-licenses.json"
if ($env:ESPEAK_NG_DIR) {
    Copy-Item "$env:ESPEAK_NG_DIR\espeak-ng.exe", "$env:ESPEAK_NG_DIR\libespeak-ng.dll" $Stage
    Copy-Item -Recurse "$env:ESPEAK_NG_DIR\espeak-ng-data" $Stage
}
Run "$QtDir\bin\windeployqt.exe" @("--release", "--qmldir", "$Root\qml", "--no-translations", "--no-system-d3d-compiler", "--no-opengl-sw", "--no-compiler-runtime", "$Stage\setu-shell.exe", "$Stage\setu-ui.exe")
foreach ($d in @("qmltooling", "virtualkeyboard", "sqldrivers", "multimedia", "networkinformation", "generic", "position", "tls")) {
    if (Test-Path "$Stage\$d") { Remove-Item -Recurse -Force "$Stage\$d" }
}
Get-ChildItem "$Stage\imageformats" -ErrorAction SilentlyContinue | Where-Object { $_.Name -notmatch "qsvg" } | Remove-Item -Force

Step "Sign binaries"
function Sign($files) {
    if ($env:SETU_SIGN_CERT) {
        Run "signtool" (@("sign", "/fd", "sha256", "/tr", "http://timestamp.digicert.com", "/td", "sha256", "/f", $env:SETU_SIGN_CERT, "/p", $env:SETU_SIGN_PASSWORD) + $files)
    } elseif ($env:AZURE_SIGNING_ENDPOINT) {
        Run "sign" (@("code", "trusted-signing", "-tse", $env:AZURE_SIGNING_ENDPOINT, "-tsa", $env:AZURE_SIGNING_ACCOUNT, "-tscp", $env:AZURE_SIGNING_PROFILE) + $files)
    } else {
        Write-Host "No signing certificate configured; artifacts are unsigned"
    }
}
Sign (Get-ChildItem -Recurse $Stage -Include *.exe, *.dll | ForEach-Object { $_.FullName })

Step "Installer"
New-Item -ItemType Directory -Force $Out | Out-Null
$Setup = "$Out\Setu-$Version-$Arch-setup.exe"
Push-Location $Here
Run "makensis" @("/V2", "/DVERSION=$Version", "/DSTAGE=$Stage", "/DOUTFILE=$Setup", "setu.nsi")
Pop-Location
Sign @($Setup)
(Get-FileHash -Algorithm SHA256 $Setup).Hash.ToLower() + "  " + (Split-Path -Leaf $Setup) | Out-File -Encoding ascii "$Setup.sha256"
Write-Host $Setup
