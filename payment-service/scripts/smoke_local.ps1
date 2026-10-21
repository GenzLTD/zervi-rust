# 本地冒烟：启动 payment-service（连本地 genzltd 52 表）验证三端点基础行为
$env:PATH = 'C:\ServBay\packages\rust\current\cargo\bin;C:\ServBay\packages\rust\current\rustc\bin;' + $env:PATH
$env:PAYMENT_PORT = '3009'
# DATABASE_URL 用 common 默认 postgres://postgres:postgres@127.0.0.1:5433/genzltd
Set-Location c:/ServBay/www/zervi-payment/payment-service

Write-Host "=== 1. 后台启动 payment-service ==="
$proc = Start-Process powershell -ArgumentList "-NoProfile","-Command","cd c:/ServBay/www/zervi-payment/payment-service; `$env:PAYMENT_PORT='3009'; `$env:PATH='C:\ServBay\packages\rust\current\cargo\bin;C:\ServBay\packages\rust\current\rustc\bin;'+`$env:PATH; cargo run -p payment-service --bin payment-service *> `$env:TEMP\payment-svc.log" -WindowStyle Hidden -PassThru
Start-Sleep -Seconds 25

Write-Host "=== 2. healthz ==="
try { $h = Invoke-WebRequest "http://127.0.0.1:3009/healthz" -UseBasicParsing -TimeoutSec 5; "HTTP $($h.StatusCode): $($h.Content)" } catch { "healthz 失败: $($_.Exception.Message)" }

Write-Host "=== 3. webhook 垃圾载荷（预期 200：结构性坏数据 ACK 掉）==="
try {
    $r = Invoke-WebRequest "http://127.0.0.1:3009/v1/payment/apple/webhook" -Method Post -Body "garbage-not-jws" -ContentType "text/plain" -UseBasicParsing -TimeoutSec 5
    "HTTP $($r.StatusCode): $($r.Content)"
} catch {
    "HTTP $($_.Exception.Response.StatusCode.value__): 已落 raw？(取决于 root CA 配置)"
}

Write-Host "=== 4. verify 无鉴权（预期 401）==="
try {
    $r = Invoke-WebRequest "http://127.0.0.1:3009/v1/payment/apple/verify" -Method Post -Body '{"transaction_id":"123"}' -ContentType "application/json" -UseBasicParsing -TimeoutSec 5
    "HTTP $($r.StatusCode): $($r.Content)"
} catch {
    "HTTP $($_.Exception.Response.StatusCode.value__)（符合预期）"
}

Write-Host "=== 5. 服务日志尾部 ==="
Get-Content "$env:TEMP\payment-svc.log" -Tail 8 -ErrorAction SilentlyContinue

Write-Host "=== 6. 停止服务 ==="
Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Get-NetTCPConnection -LocalPort 3009 -State Listen -ErrorAction SilentlyContinue | ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }
"done"
