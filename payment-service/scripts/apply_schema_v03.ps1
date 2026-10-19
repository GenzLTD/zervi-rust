$env:PGPASSWORD = 'postgres'
$psql = 'C:\ServBay\packages\postgresql\current\bin\psql.exe'

Write-Host "=== 1. 清掉现有 IAP 两表（v0.3 会重建，保证基准一次性成型）==="
& $psql -h 127.0.0.1 -p 5433 -U postgres -d genzltd -c 'DROP TABLE IF EXISTS apple_iap_transactions; DROP TABLE IF EXISTS iap_subscriptions;'

Write-Host "=== 2. 执行 v0.3 全量基准 ==="
& $psql -h 127.0.0.1 -p 5433 -U postgres -d genzltd -v ON_ERROR_STOP=1 -f 'c:/ServBay/www/zervi-payment/shared/genzltd_schema_v0.3.sql' 2>&1 | Select-String 'CREATE TABLE|CREATE INDEX|ERROR' | Measure-Object | ForEach-Object { "DDL 语句执行数: $($_.Count)" }

Write-Host "=== 3. 验证：表数 / RLS / 关键表抽查 ==="
& $psql -h 127.0.0.1 -p 5433 -U postgres -d genzltd -c "select count(*) as tables, count(*) filter (where rowsecurity) as rls_tables from pg_tables where schemaname='public';"
& $psql -h 127.0.0.1 -p 5433 -U postgres -d genzltd -c "select column_name, data_type, character_maximum_length from information_schema.columns where table_name='iap_subscriptions' order by ordinal_position limit 8;"
& $psql -h 127.0.0.1 -p 5433 -U postgres -d genzltd -c "select count(*) as fk_count from pg_constraint where contype='f' and connamespace='public'::regnamespace;"
