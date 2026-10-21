$env:PATH = 'C:\ServBay\packages\rust\current\cargo\bin;C:\ServBay\packages\rust\current\rustc\bin;' + $env:PATH
Set-Location c:/ServBay/www/zervi-payment
Write-Host "=== cargo test -p payment-service（含 v0.4 routes + 全 workspace 编译检查）==="
cargo test -p payment-service 2>&1 | Select-Object -Last 30
