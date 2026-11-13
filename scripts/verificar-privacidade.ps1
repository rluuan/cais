# Confere, no executável já compilado, que o Cais não importa nenhuma DLL de rede.
# Uso:  .\scripts\verificar-privacidade.ps1 [caminho\do\cais.exe]
param([string]$Exe = "$PSScriptRoot\..\target\release\cais.exe")

$ErrorActionPreference = "Stop"
if (-not (Test-Path $Exe)) { throw "Executável não encontrado: $Exe (rode cargo build --release)" }

# Lê a tabela de importação direto do arquivo PE, sem depender de ferramentas externas.
$b = [System.IO.File]::ReadAllBytes((Resolve-Path $Exe))
function U16($o) { [BitConverter]::ToUInt16($b, $o) }
function U32($o) { [BitConverter]::ToUInt32($b, $o) }
$pe = U32 0x3C
$opt = $pe + 24
$secoes = U16 ($pe + 6)
$tamOpt = U16 ($pe + 20)
$importRva = U32 ($opt + 120)   # PE32+: diretório 1 (imports)
$tabSec = $opt + $tamOpt
function RvaParaOffset($rva) {
  for ($i = 0; $i -lt $secoes; $i++) {
    $s = $tabSec + 40 * $i
    $va = U32 ($s + 12); $tam = U32 ($s + 8); $raw = U32 ($s + 20)
    if ($rva -ge $va -and $rva -lt $va + [Math]::Max($tam, (U32 ($s + 16)))) { return $rva - $va + $raw }
  }
  throw "RVA fora das seções"
}
$dlls = @()
$o = RvaParaOffset $importRva
while ($true) {
  $nomeRva = U32 ($o + 12)
  if ($nomeRva -eq 0) { break }
  $p = RvaParaOffset $nomeRva; $fim = $p
  while ($b[$fim] -ne 0) { $fim++ }
  $dlls += [Text.Encoding]::ASCII.GetString($b, $p, $fim - $p).ToLower()
  $o += 20
}

$rede = "ws2_32.dll","wsock32.dll","winhttp.dll","wininet.dll","urlmon.dll","dnsapi.dll","iphlpapi.dll","mswsock.dll","webio.dll","httpapi.dll","secur32.dll","schannel.dll","ncrypt.dll","crypt32.dll"
Write-Host "DLLs importadas pelo Cais:" -ForegroundColor Cyan
$dlls | Sort-Object | ForEach-Object { Write-Host "  $_" }
$achadas = $dlls | Where-Object { $rede -contains $_ -or $_ -like "api-ms-win-core-winsock*" }
if ($achadas) {
  Write-Host "FALHOU: o executável importa DLL de rede: $($achadas -join ', ')" -ForegroundColor Red
  exit 1
}
Write-Host "OK: nenhuma DLL de rede importada." -ForegroundColor Green
