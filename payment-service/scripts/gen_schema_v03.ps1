$env:PATH = 'C:\ServBay\packages\rust\current\cargo\bin;C:\ServBay\packages\rust\current\rustc\bin;' + $env:PATH
Set-Location c:/ServBay/www/zervi-payment

$body = Join-Path $env:TEMP 'genzltd_body.sql'
$py   = 'C:\ServBay\packages\python\current\python.exe'
if (-not (Test-Path $py)) { $py = 'python' }

Write-Host "=== 1. gen_schema 生成 50 表 DDL ==="
cargo run -p payment-service --bin gen_schema 2>$null | Out-File -Encoding utf8 $body
if ($LASTEXITCODE -ne 0) { Get-Content $body -Tail 20; exit 1 }
Write-Host "body: $((Get-Item $body).Length) bytes"

Write-Host "=== 2. 拓扑排序 + 拼装 v0.3 ==="
& $py payment-service/scripts/assemble_schema_v03.py $body payment-service/schema_v0.1.sql shared/genzltd_schema_v0.3.sql

Write-Host "=== 3. 结果概览 ==="
(Get-Content shared/genzltd_schema_v0.3.sql | Select-String 'CREATE TABLE').Count
