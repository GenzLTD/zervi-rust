$env:PGPASSWORD = 'postgres'
$psql = 'C:\ServBay\packages\postgresql\current\bin\psql.exe'

Write-Host "=== 1. 创建 genzltd 库 ==="
& $psql -h 127.0.0.1 -p 5433 -U postgres -d postgres -c 'CREATE DATABASE genzltd;' 2>&1

Write-Host "=== 2. 执行 schema_v0.1.sql ==="
& $psql -h 127.0.0.1 -p 5433 -U postgres -d genzltd -v ON_ERROR_STOP=1 -f 'c:/ServBay/www/zervi-payment/payment-service/schema_v0.1.sql' 2>&1

Write-Host "=== 3. 验证表与 RLS ==="
& $psql -h 127.0.0.1 -p 5433 -U postgres -d genzltd -c "select tablename, rowsecurity from pg_tables where schemaname='public' order by tablename;"
