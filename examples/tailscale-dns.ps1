# Prepares Windows so Tailscale can log in where plain DNS (port 53) is refused
# (found on the owner's PC 2026-10-30: "failed to resolve controlplane.tailscale.com").
# Run in PowerShell as administrator:  powershell -File examples\tailscale-dns.ps1
# Undo: powershell -File examples\tailscale-dns.ps1 -Undo
param([switch]$Undo)
$ErrorActionPreference = 'Stop'
$adapters = 'Ethernet', 'WiFi 2'   # names from Get-NetAdapter on that PC

if ($Undo) {
  Set-DnsClientServerAddress -InterfaceAlias $adapters -ResetServerAddresses
  foreach ($ip in '1.1.1.1', '8.8.8.8') {
    Set-DnsClientDohServerAddress -ServerAddress $ip -AllowFallbackToUdp $true -AutoUpgrade $false
  }
} else {
  Set-DnsClientServerAddress -InterfaceAlias $adapters -ServerAddresses '1.1.1.1', '8.8.8.8'
  # Encrypted DNS only, no fallback to the refused port 53.
  if (-not (Get-DnsClientDohServerAddress -ServerAddress 1.1.1.1 -ErrorAction SilentlyContinue)) {
    Add-DnsClientDohServerAddress -ServerAddress 1.1.1.1 -DohTemplate 'https://cloudflare-dns.com/dns-query' -AllowFallbackToUdp $false -AutoUpgrade $true
  } else {
    Set-DnsClientDohServerAddress -ServerAddress 1.1.1.1 -AllowFallbackToUdp $false -AutoUpgrade $true
  }
  Set-DnsClientDohServerAddress -ServerAddress 8.8.8.8 -AllowFallbackToUdp $false -AutoUpgrade $true
}
Clear-DnsClientCache
Restart-Service Tailscale -Force
# ASCII only: Windows PowerShell 5.1 reads a BOM-less script in the ANSI code page.
'Done. Next: Tailscale tray icon -> Log in, then: local-photo --tunnel funnel'
