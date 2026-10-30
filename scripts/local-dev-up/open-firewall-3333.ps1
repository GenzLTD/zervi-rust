# 栈主机：放行局域网访问 api-gateway :3333（需管理员）。幂等。
# 用法：以管理员打开 PowerShell 后
#   powershell -File scripts/local-dev-up/open-firewall-3333.ps1

$ErrorActionPreference = "Stop"
$name = "zervi-local-dev-up-gateway-3333"

$existing = Get-NetFirewallRule -DisplayName $name -ErrorAction SilentlyContinue
if ($existing) {
  Write-Host "已存在规则: $name"
  Get-NetFirewallRule -DisplayName $name | Get-NetFirewallPortFilter
  exit 0
}

New-NetFirewallRule `
  -DisplayName $name `
  -Direction Inbound `
  -Action Allow `
  -Protocol TCP `
  -LocalPort 3333 `
  -Profile Private, Domain `
  -Description "local-dev-up: allow LAN clients (Surface/Mac/Dell) to hit api-gateway" | Out-Null

Write-Host "已添加入站规则: $name (TCP 3333, Private+Domain)"
Write-Host "公网/Public 配置文件未放行；需要时请手工改 Profile。"
