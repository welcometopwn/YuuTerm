param()
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$version='0.20.2'
$archiveHash='975A22143262FD27476D19EE00C7AF67978426E40E1DEE94EED6BBADE1CF87DC'
$exeHash='F7292FAB58C706638C999E64C4BA82E5128AE628130BA55E3266A768EE431FBF'
$tools=Join-Path $root 'target/dependency-tools'
$tool=Join-Path $tools "cargo-deny-$version-x86_64-pc-windows-msvc/cargo-deny.exe"
$folder=Join-Path $root ('target/dependency-review/run-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$utf8=New-Object Text.UTF8Encoding($false)
Push-Location $root
try {
    New-Item -ItemType Directory -Force $tools,$folder | Out-Null
    if(-not(Test-Path -LiteralPath $tool)) {
        $archive=Join-Path $tools 'cargo-deny.tar.gz'
        Invoke-WebRequest "https://github.com/EmbarkStudios/cargo-deny/releases/download/$version/cargo-deny-$version-x86_64-pc-windows-msvc.tar.gz" -OutFile $archive
        if((Get-FileHash -LiteralPath $archive).Hash -ne $archiveHash){throw 'cargo-deny archive checksum mismatch'}
        & tar.exe -xzf $archive -C $tools
        if($LASTEXITCODE){throw 'cargo-deny extraction failed'}
    }
    if((Get-FileHash -LiteralPath $tool).Hash -ne $exeHash){throw 'cargo-deny executable checksum mismatch'}
    # Native stderr is diagnostic output, not a PowerShell terminating error.
    $ErrorActionPreference='Continue'
    $rsaReview=@(& node.exe scripts/review-rsa-exception.mjs 2>&1 | ForEach-Object {"$_"})
    $rsaExit=$LASTEXITCODE
    $reviewConfig=Join-Path $root 'deny.toml'
    if ($rsaExit -eq 0) {
        # The pinned cargo-deny has no per-dependent/absolute-date exceptions.
        # Only after the mandatory graph/source/date guard succeeds, generate a
        # run-local policy for this exact graph. The global policy stays strict.
        $policy=[IO.File]::ReadAllText($reviewConfig)
        if(-not $policy.Contains('ignore = []')) { throw 'Unexpected global advisory suppression' }
        $policy=$policy.Replace('ignore = []','ignore = [{ id = "RUSTSEC-2023-0071", reason = "Guarded accepted risk: exact reviewed graph/source; expires 2026-11-19 UTC; docs/rsa-advisory-review.md" }]')
        $policy=$policy.Replace('target/dependency-review/advisory-dbs',((Join-Path $root 'target/dependency-review/advisory-dbs').Replace('\','/')))
        $reviewConfig=Join-Path $folder 'deny-reviewed.toml'
        [IO.File]::WriteAllText($reviewConfig,$policy,$utf8)
    }
    $install=@(& npm.cmd ci --ignore-scripts 2>&1 | ForEach-Object {"$_"})
    $installExit=$LASTEXITCODE
    $deny=@(& $tool --locked --config $reviewConfig --color never --format json check advisories licenses sources bans --show-stats 2>&1 | ForEach-Object {"$_"})
    $denyExit=$LASTEXITCODE
    $inventory=@(& $tool --locked list --format json --layout crate 2>&1 | ForEach-Object {"$_"})
    $inventoryExit=$LASTEXITCODE
    $audit=@(& npm.cmd audit --json 2>&1 | ForEach-Object {"$_"})
    $auditExit=$LASTEXITCODE
    $licenses=@(& node.exe scripts/review-npm-licenses.mjs 2>&1 | ForEach-Object {"$_"})
    $licenseExit=$LASTEXITCODE
    $ErrorActionPreference='Stop'
    [IO.File]::WriteAllText((Join-Path $folder 'rsa-exception.json'),($rsaReview -join "`n")+"`n",$utf8)
    [IO.File]::WriteAllText((Join-Path $folder 'npm-ci.txt'),($install -join "`n")+"`n",$utf8)
    [IO.File]::WriteAllText((Join-Path $folder 'cargo-deny.jsonl'),($deny -join "`n")+"`n",$utf8)
    [IO.File]::WriteAllText((Join-Path $folder 'rust-licenses.json'),($inventory -join "`n")+"`n",$utf8)
    [IO.File]::WriteAllText((Join-Path $folder 'npm-audit.json'),($audit -join "`n")+"`n",$utf8)
    [IO.File]::WriteAllText((Join-Path $folder 'npm-licenses.json'),($licenses -join "`n")+"`n",$utf8)
    $database=Get-ChildItem target/dependency-review/advisory-dbs -Directory | Select-Object -First 1
    $revision=if($database){& git -C $database.FullName rev-parse HEAD}else{$null}
    $summary=@{rsa_exception_exit=$rsaExit;cargo_deny_version=$version;rust_inventory_exit=$inventoryExit;npm_ci_exit=$installExit;cargo_deny_exit=$denyExit;npm_audit_exit=$auditExit;npm_licenses_exit=$licenseExit;advisory_database_commit=$revision;utc=[DateTime]::UtcNow.ToString('o');cargo_lock_sha256=(Get-FileHash Cargo.lock).Hash;npm_lock_sha256=(Get-FileHash package-lock.json).Hash;passed=($rsaExit -eq 0 -and $inventoryExit -eq 0 -and $installExit -eq 0 -and $denyExit -eq 0 -and $auditExit -eq 0 -and $licenseExit -eq 0)}
    [IO.File]::WriteAllText((Join-Path $folder 'summary.json'),($summary|ConvertTo-Json)+"`n",$utf8)
    $summary|ConvertTo-Json
    "Reports: $folder"
    if(-not $summary.passed){exit 1}
}finally{Pop-Location}
