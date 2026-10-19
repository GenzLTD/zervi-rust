# 探测云端 Supabase：表清单 + IAP 表是否存在（凭据从环境变量取，不硬编码）
$url = $env:SUPABASE_URL
$key = $env:SUPABASE_SECRET_KEY
if (-not $url -or -not $key) { Write-Host "缺少 SUPABASE_URL / SUPABASE_SECRET_KEY 环境变量"; exit 1 }

$tmp = Join-Path $env:TEMP "supabase_openapi.json"
Write-Host "=== 1. REST OpenAPI 探测（验证凭据与连通性）==="
curl.exe -s -o $tmp "$url/rest/v1/" -H "apikey: $key" -H "Authorization: Bearer $key"
Write-Host "HTTP body size: $((Get-Item $tmp).Length) bytes"

Write-Host "=== 2. public 表清单 ==="
$py = 'C:\ServBay\packages\python\current\python.exe'
if (-not (Test-Path $py)) { $py = 'python' }
& $py -c "import json,os; d=json.load(open(os.environ['TEMP']+r'\supabase_openapi.json',encoding='utf-8')); defs=sorted(d.get('definitions',{}).keys()); print(f'{len(defs)} tables:'); [print(' -',t) for t in defs]"
