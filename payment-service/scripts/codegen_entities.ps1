$cli = 'D:\DevTools\.cargo\bin\sea-orm-cli.exe'
$out = 'c:/ServBay/www/zervi-payment/payment-service/entity_codegen'

Write-Host "=== sea-orm-codegen（genzltd -> entity_codegen/）==="
& $cli generate entity `
    -u 'postgres://postgres:postgres@127.0.0.1:5433/genzltd' `
    -o $out `
    --with-serde both 2>&1

Write-Host "=== 生成结果 ==="
Get-ChildItem $out -Recurse -File | Select-Object -ExpandProperty FullName
