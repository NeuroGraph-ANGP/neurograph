<#
.SYNOPSIS
    NeuroGraph v3.5.2 — Multi-node Test Runner
    Ruleaza 2 teste cu cate 20 noduri fiecare:
      Test 1: 20 noduri honest (fara atacatori)
      Test 2: 10 noduri honest + 10 atacatori (diversi)

.DESCRIPTION
    Pentru fiecare nod:
      - Porneste o fereastra PowerShell separata
      - Afiseaza PoW mining, nume, peer list, attack type
      - La fiecare 500 de pasi: reputatii, tranzactii finalizate,
        mempool size, TPS, clock skew stats, per-node details
      - Simularea se opreste dupa 5000 pasi (aprox. 4 minute)

.NOTES
    File:    run_neurograph_tests.ps1
    Author:  Z.ai Labs
    Version: 1.0 for NeuroGraph v3.5.2
    Requires: Rust 1.96+ (cargo), PowerShell 5.1+ or PowerShell 7+
#>

param(
    [string]$ProjectDir = "$env:USERPROFILE\Desktop\neurograph_v3.5_final",
    [int]$BasePort = 20000,
    [int]$NodeCount = 20,
    [switch]$SkipBuild,
    [switch]$Test1Only,
    [switch]$Test2Only
)

$ErrorActionPreference = "Stop"

# ─── Helpers ─────────────────────────────────────────────────────────
function Write-Header($title) {
    $line = "=" * 70
    Write-Host ""
    Write-Host $line -ForegroundColor Cyan
    Write-Host "  $title" -ForegroundColor White
    Write-Host $line -ForegroundColor Cyan
}

function Test-CargoAvailable {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) {
        Write-Host "[ERROR] cargo nu e in PATH. Instaleaza Rust de la https://rustup.rs" -ForegroundColor Red
        exit 1
    }
    Write-Host "[OK] Cargo gasit: $($cargo.Source)" -ForegroundColor Green
}

function Build-Neurograph {
    Write-Header "BUILD NeuroGraph v3.5.2 (release)"
    Push-Location $ProjectDir
    try {
        & cargo build --release 2>&1 | Out-Host
        if ($LASTEXITCODE -ne 0) {
            Write-Host "[ERROR] Build a esuat." -ForegroundColor Red
            exit 1
        }
        $bin = Join-Path $ProjectDir "target\release\angp.exe"
        if (-not (Test-Path $bin)) {
            Write-Host "[ERROR] Binarul nu a fost generat: $bin" -ForegroundColor Red
            exit 1
        }
        $size = (Get-Item $bin).Length / 1KB
        Write-Host "[OK] Binar: $bin ($([math]::Round($size,1)) KB)" -ForegroundColor Green
    }
    finally {
        Pop-Location
    }
}

function Start-Node {
    param(
        [string]$Name,
        [int]$Port,
        [string]$AttackType,
        [string[]]$Peers,
        [string]$LogFile
    )
    $bin = Join-Path $ProjectDir "target\release\angp.exe"
    $args = @("--port", $Port, "--name", $Name, "--attack-type", $AttackType)
    foreach ($p in $Peers) {
        $args += @("--peer", $p)
    }

    # Window title + command (cd to project dir so data/ goes there)
    $cmd = "cd '$ProjectDir'; Write-Host '=== NeuroGraph v3.5.2 — $Name ===' -ForegroundColor Cyan; Write-Host 'Attack: $AttackType  Port: $Port  Peers: $($Peers.Count)' -ForegroundColor Yellow; Write-Host ''; & '$bin' $($args -join ' ') 2>&1 | Tee-Object -FilePath '$LogFile'"
    $psArgs = @("-NoExit", "-Command", $cmd)

    $proc = Start-Process -FilePath "powershell.exe" -ArgumentList $psArgs -PassThru -WindowStyle Normal
    return $proc
}

function New-PeerList {
    param([int]$MyIndex, [int]$Total, [int]$BasePort)
    # Full mesh: toti ceilalti noduri ca peers
    $peers = @()
    for ($i = 0; $i -lt $Total; $i++) {
        if ($i -ne $MyIndex) {
            $peers += "127.0.0.1:$($BasePort + $i)"
        }
    }
    return $peers
}

# ─── TEST 1: 20 honest nodes ────────────────────────────────────────
function Run-Test1 {
    Write-Header "TEST 1: 20 noduri HONEST (fara atacatori)"
    Write-Host "Topologie: full mesh (fiecare nod cu 19 peers)" -ForegroundColor Yellow
    Write-Host "Porturi: $BasePort .. $($BasePort + $NodeCount - 1)" -ForegroundColor Yellow
    Write-Host "Simulare: 5000 pasi, raport la fiecare 500 pasi" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Apasa ENTER pentru a porni cele 20 ferestre..." -ForegroundColor Yellow
    Read-Host

    # Creeaza folder logs
    $logDir = Join-Path $ProjectDir "logs\test1"
    if (Test-Path $logDir) { Remove-Item $logDir -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $logDir | Out-Null

    # Pornire noduri
    $processes = @()
    for ($i = 0; $i -lt $NodeCount; $i++) {
        $name = "honest_node_$($i.ToString('D2'))"
        $port = $BasePort + $i
        $peers = New-PeerList -MyIndex $i -Total $NodeCount -BasePort $BasePort
        $log = Join-Path $logDir "$name.log"
        Write-Host "  Starting $name on port $port (attack: honest, peers: $($peers.Count))" -ForegroundColor Green
        $proc = Start-Node -Name $name -Port $port -AttackType "honest" -Peers $peers -LogFile $log
        $processes += $proc
        Start-Sleep -Milliseconds 200  # Stagger start ca sa evit port collisions
    }

    Write-Host ""
    Write-Host "[OK] 20 noduri pornite. Fiecare fereastra afiseaza:" -ForegroundColor Green
    Write-Host "     - PoW mining (SHA-512/256 difficulty)"
    Write-Host "     - Node name, port, attack type, peer list"
    Write-Host "     - La fiecare 500 pasi: reputatii, finalized txs, mempool, TPS, clock skew"
    Write-Host ""
    Write-Host "Simularea se va opri automat dupa 5000 pasi (~4 minute)" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Pentru a opri fortat toate nodurile, ruleaza:" -ForegroundColor Yellow
    Write-Host "  Get-Process angp -ErrorAction SilentlyContinue | Stop-Process -Force" -ForegroundColor White
    Write-Host ""

    Write-Host "Logs salvate in: $logDir" -ForegroundColor Cyan
    Write-Host ""
    return $processes
}

# ─── TEST 2: 10 honest + 10 attackers ───────────────────────────────
function Run-Test2 {
    Write-Header "TEST 2: 10 HONEST + 10 ATACATORI (diversi)"
    Write-Host "Topologie: full mesh (fiecare nod cu 19 peers)" -ForegroundColor Yellow
    Write-Host "Porturi: $BasePort .. $($BasePort + $NodeCount - 1)" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Distributie atacatori:" -ForegroundColor Yellow
    Write-Host "  nodes 00-09: honest       (10 noduri)"
    Write-Host "  nodes 10-11: coordinated  (2 noduri, atac coordonat)"
    Write-Host "  nodes 12-13: clone        (2 noduri, copiaza alt peer)"
    Write-Host "  nodes 14-15: adaptive     (2 noduri, atac adaptiv)"
    Write-Host "  nodes 16-17: sybil        (2 noduri, zgomot mic)"
    Write-Host "  nodes 18-19: flipflop     (2 noduri, oscilare)"
    Write-Host ""
    Write-Host "Apasa ENTER pentru a porni cele 20 ferestre..." -ForegroundColor Yellow
    Read-Host

    $logDir = Join-Path $ProjectDir "logs\test2"
    if (Test-Path $logDir) { Remove-Item $logDir -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $logDir | Out-Null

    # Definim attack types per nod
    $attackTypes = @(
        "honest","honest","honest","honest","honest",
        "honest","honest","honest","honest","honest",
        "coordinated","coordinated",
        "clone","clone",
        "adaptive","adaptive",
        "sybil","sybil",
        "flipflop","flipflop"
    )

    $prefixes = @(
        "honest_","honest_","honest_","honest_","honest_",
        "honest_","honest_","honest_","honest_","honest_",
        "coord_","coord_",
        "clone_","clone_",
        "adapt_","adapt_",
        "sybil_","sybil_",
        "flip_","flip_"
    )

    $processes = @()
    for ($i = 0; $i -lt $NodeCount; $i++) {
        $name = "$($prefixes[$i])$($i.ToString('D2'))"
        $port = $BasePort + $i
        $attack = $attackTypes[$i]
        $peers = New-PeerList -MyIndex $i -Total $NodeCount -BasePort $BasePort
        $log = Join-Path $logDir "$name.log"
        $color = if ($attack -eq "honest") { "Green" } else { "Red" }
        Write-Host "  Starting $name on port $port (attack: $attack, peers: $($peers.Count))" -ForegroundColor $color
        $proc = Start-Node -Name $name -Port $port -AttackType $attack -Peers $peers -LogFile $log
        $processes += $proc
        Start-Sleep -Milliseconds 200
    }

    Write-Host ""
    Write-Host "[OK] 20 noduri pornite (10 honest + 10 atacatori)." -ForegroundColor Green
    Write-Host ""
    Write-Host "Ce sa urmaresti in ferestre:" -ForegroundColor Yellow
    Write-Host "  - Nodurile honest ar trebui sa pastreze reputatie > 0.7"
    Write-Host "  - Nodurile coordonate/clone ar trebui sa scada sub 0.3"
    Write-Host "  - Adaptive incearca sa se ajusteze, poate ramana ~0.5"
    Write-Host "  - Flipflop va oscila in functie de pas"
    Write-Host ""
    Write-Host "La fiecare 500 pasi vei vedea tabelul per-node:" -ForegroundColor Cyan
    Write-Host "  Node | Rep | Messages | Distance | Status"
    Write-Host "  Status: OK (>0.7) | WARN (>0.3) | BAD (<=0.3)"
    Write-Host ""
    Write-Host "Simularea se opreste dupa 5000 pasi (~4 minute)" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Logs: $logDir" -ForegroundColor Cyan
    Write-Host ""
    return $processes
}

# ─── MAIN ────────────────────────────────────────────────────────────
Write-Header "NeuroGraph v3.5.2 — Multi-Node Test Runner"
Write-Host "Project dir: $ProjectDir" -ForegroundColor White
Write-Host "Base port:    $BasePort" -ForegroundColor White
Write-Host "Node count:   $NodeCount per test" -ForegroundColor White
Write-Host ""

if (-not (Test-Path $ProjectDir)) {
    Write-Host "[ERROR] Project dir nu exista: $ProjectDir" -ForegroundColor Red
    Write-Host "        Seteaza -ProjectDir sau copiaza proiectul acolo." -ForegroundColor Yellow
    exit 1
}

Test-CargoAvailable

if (-not $SkipBuild) {
    Build-Neurograph
} else {
    Write-Host "[SKIP] Build sarit (foloseste binar existent)" -ForegroundColor Yellow
}

$p1 = @()
$p2 = @()

if (-not $Test2Only) {
    $p1 = Run-Test1
    Write-Host "Asteapta terminarea TEST 1 (sau inchide ferestrele manual)..." -ForegroundColor Yellow
    Write-Host "Apoi apasa ENTER pentru TEST 2." -ForegroundColor Yellow
    Read-Host
}

if (-not $Test1Only) {
    # Opreste nodurile din test 1 daca mai ruleaza
    Write-Host "Opresc nodurile din TEST 1..." -ForegroundColor Yellow
    Get-Process angp -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    $p2 = Run-Test2
}

Write-Header "FINAL"
Write-Host "Ambele teste au fost pornite." -ForegroundColor Green
Write-Host ""
Write-Host "Rezultate detaliate in:" -ForegroundColor Cyan
Write-Host "  $($ProjectDir)\logs\test1\*.log"
Write-Host "  $($ProjectDir)\logs\test2\*.log"
Write-Host ""
Write-Host "Pentru a opri toate nodurile:" -ForegroundColor Yellow
Write-Host "  Get-Process angp -ErrorAction SilentlyContinue | Stop-Process -Force"
Write-Host ""

# Asteapta ca toate procesele sa termine
if ($p1.Count -gt 0) {
    Write-Host "Astept terminarea nodurilor TEST 1..." -ForegroundColor Yellow
    $p1 | ForEach-Object { $_.WaitForExit() } -ErrorAction SilentlyContinue
}
if ($p2.Count -gt 0) {
    Write-Host "Astept terminarea nodurilor TEST 2..." -ForegroundColor Yellow
    $p2 | ForEach-Object { $_.WaitForExit() } -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "[DONE] Toate testele au fost finalizate." -ForegroundColor Green
