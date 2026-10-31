# 本机联调抽检（经 :3333）
$ErrorActionPreference = "Stop"
$env:NO_PROXY = "127.0.0.1,localhost,::1"
$Base = if ($env:BASE) { $env:BASE } else { "http://127.0.0.1:3333" }
$Pass = 0; $Fail = 0

function Ok($m) { Write-Host "  ok  $m"; $script:Pass++ }
function Bad($m) { Write-Host "  FAIL $m"; $script:Fail++ }

function Req([string]$Method, [string]$Path, [string]$Token = "", $Body = $null) {
  $headers = @{ Accept = "application/json" }
  if ($Token) { $headers.Authorization = "Bearer $Token" }
  $params = @{
    Uri             = "$Base$Path"
    Method          = $Method
    Headers         = $headers
    UseBasicParsing = $true
    TimeoutSec      = 30
  }
  if ($null -ne $Body) {
    $params.ContentType = "application/json; charset=utf-8"
    $params.Body = ($Body | ConvertTo-Json -Compress -Depth 8)
  }
  try {
    $resp = Invoke-WebRequest @params
    $raw = $resp.Content
    if ($raw -is [byte[]]) { $raw = [System.Text.Encoding]::UTF8.GetString($raw) }
    return @{ Code = [int]$resp.StatusCode; Body = [string]$raw }
  } catch {
    $r = $_.Exception.Response
    if ($r) {
      $code = [int]$r.StatusCode
      $reader = New-Object System.IO.StreamReader($r.GetResponseStream())
      $text = $reader.ReadToEnd()
      return @{ Code = $code; Body = [string]$text }
    }
    return @{ Code = 0; Body = "$_" }
  }
}

Write-Host "== smoke_check @ $Base =="

$h = Req GET /health
if ($h.Code -eq 200) { Ok "gateway /health" } else { Bad "gateway /health $($h.Code)" }

$email = "dev-$(Get-Date -Format 'yyyyMMddHHmmss')@local.test"
$reg = Req POST /v1/auth/register "" @{ email = $email; password = "Passw0rd!"; name = "dev" }
if ($reg.Code -in 200, 201, 409) { Ok "register $($reg.Code)" } else { Bad "register $($reg.Code) $($reg.Body)" }

$login = Req POST /v1/auth/login "" @{ email = $email; password = "Passw0rd!" }
$token = $null
try {
  $obj = $login.Body | ConvertFrom-Json
  $token = $obj.token
  if (-not $token) { $token = $obj.access_token }
} catch {}
if ($login.Code -eq 200 -and $token) { Ok "login token" } else {
  Bad "login $($login.Code) $($login.Body)"; Write-Host "PASS=$Pass FAIL=$Fail"; exit 1
}

$g = Req POST /v1/compliance/consent/grant $token @{ scope = "jobseeker_core" }
if ($g.Code -eq 200) { Ok "consent grant" } else { Bad "consent grant $($g.Code) $($g.Body)" }

$st = Req GET /v1/compliance/consent/status $token
$flag = $false
try { $flag = [bool](($st.Body | ConvertFrom-Json).status.jobseeker_core) } catch {}
if ($st.Code -eq 200 -and $flag) { Ok "consent status jobseeker_core=true" }
else { Bad "consent status $($st.Code) $($st.Body)" }

$t = Req GET /v1/resume/templates $token
if ($t.Code -eq 200) { Ok "templates" } else { Bad "templates $($t.Code)" }

$ing = Req POST /v1/resume/ingest $token @{ markdown = "# local-dev-up`nsmoke"; filename = "smoke.md" }
$rid = $null
try { $rid = ($ing.Body | ConvertFrom-Json).resume_id } catch {}
if ($ing.Code -eq 200 -and $rid) { Ok "ingest resume_id=$rid" } else { Bad "ingest $($ing.Code) $($ing.Body)" }

$l = Req GET /v1/resume/list $token
if ($l.Code -eq 200) { Ok "list" } else { Bad "list $($l.Code)" }

if ($rid) {
  $d = Req GET "/v1/resume/$rid" $token
  if ($d.Code -eq 200) { Ok "detail" } else { Bad "detail $($d.Code)" }
  $p = Req GET "/v1/resume/$rid/permission" $token
  if ($p.Code -eq 200) { Ok "permission" } else { Bad "permission $($p.Code)" }
}

$j = Req GET /v1/jobs/list $token
if ($j.Code -eq 200) { Ok "jobs/list 200" }
else { Bad "jobs/list 期望 200 得 $($j.Code) $($j.Body)" }

Write-Host ""
Write-Host "PASS=$Pass FAIL=$Fail"
if ($Fail -ne 0) { exit 1 }
