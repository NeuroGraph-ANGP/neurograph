# ════════════════════════════════════════════════════════════════════
# NeuroGraph v3.5.12 — GitHub Repository Setup Script
# ════════════════════════════════════════════════════════════════════
# Rulează acest script DUPĂ ce ai creat repo-ul pe GitHub și ai clonat
# repo-ul gol pe calculatorul tău.
#
# Usage:
#   1. Creează repo pe GitHub: https://github.com/new
#      - Name: neurograph
#      - Description: "Adaptive Neural Gossip Protocol — first cryptocurrency with neural-inspired emergent consensus"
#      - Public
#      - NO README, NO .gitignore, NO license (le avem noi)
#   2. Clonează: git clone https://github.com/YOUR_USERNAME/neurograph.git
#   3. Copiază TOATE fișierele din neurograph_github_ready/ în neurograph/
#   4. Rulează acest script:
#        cd neurograph
#        .\init_github.ps1
# ════════════════════════════════════════════════════════════════════

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "════════════════════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host "  NeuroGraph v3.5.12 — GitHub Setup" -ForegroundColor Cyan
Write-Host "════════════════════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host ""

# Verificam ca suntem într-un git repo
if (-not (Test-Path ".git")) {
    Write-Host "ERROR: Nu suntem într-un git repo." -ForegroundColor Red
    Write-Host "Rulează: git clone https://github.com/YOUR_USERNAME/neurograph.git" -ForegroundColor Yellow
    Write-Host "Apoi: cd neurograph" -ForegroundColor Yellow
    Write-Host "Apoi: copiază fișierele din neurograph_github_ready/ aici" -ForegroundColor Yellow
    exit 1
}

# Verificam ca git e instalat
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Host "ERROR: git nu este instalat. Descarcă de la https://git-scm.com" -ForegroundColor Red
    exit 1
}

Write-Host "  Git version: $(git --version)" -ForegroundColor Green

# Configuram git daca nu e configurat
$userName = git config user.name
$userEmail = git config user.email
if (-not $userName) {
    $userName = Read-Host "  Introdu numele tău pentru git (ex: John Doe)"
    git config user.name $userName
}
if (-not $userEmail) {
    $userEmail = Read-Host "  Introdu email-ul tău pentru git"
    git config user.email $userEmail
}
Write-Host "  Git user: $userName <$userEmail>" -ForegroundColor Green
Write-Host ""

# Adaugam toate fișierele
Write-Host "─── Adăugare fișiere ──────────────────────────────────────────" -ForegroundColor Yellow
git add -A
$status = git status --short
$fileCount = ($status | Measure-Object -Line).Lines
Write-Host "  $fileCount fișiere adăugate" -ForegroundColor Green

# Primul commit
Write-Host ""
Write-Host "─── Commit inițial ────────────────────────────────────────────" -ForegroundColor Yellow
git commit -m "v3.5.12: Initial release — Adaptive Neural Gossip Protocol

NeuroGraph (ANGP) is the first cryptocurrency with neural-inspired
emergent consensus. No staking, no leader election, no voting rounds.

Key features:
- Hebbian Adaptive DAG for emergent consensus
- Reputation-weighted median voting (dual-EMA + floor)
- 961 shards with 3-level hybrid consensus
- 5-layer attack detection (12 attack types tested)
- Ed25519 batch verification (96K sigs/sec on 4 cores)
- 13,108 TPS in simulator (1K nodes, 961 shards)
- 127 test scenarios, 10/10 honest survival in all

v3.5.12 fixes the 'honest0 dies' bug present since v1.0:
- node.rs now includes self.id in get_all_reputations()
- node.rs now updates own reputation in update_reputations()
- main.rs now calls add_own_proposal() after build_proposal()

All 74 test functions pass. All 127 scenarios: 10/10 honest alive.

Codename: 'honest0 lives'"

Write-Host "  Commit creat" -ForegroundColor Green

# Tag v3.5.12
Write-Host ""
Write-Host "─── Tag v3.5.12 ──────────────────────────────────────────────" -ForegroundColor Yellow
git tag -a v3.5.12 -m "Release v3.5.12 — honest0 lives

- Self-observation fix (node.rs + main.rs)
- 127 test scenarios, 10/10 honest survival
- 96K sigs/sec, 30K pipeline TPS, 13K simulator TPS
- Whitepaper: 48 pages
- 74 test functions, all passing"

Write-Host "  Tag v3.5.12 creat" -ForegroundColor Green

# Push
Write-Host ""
Write-Host "─── Push către GitHub ────────────────────────────────────────" -ForegroundColor Yellow
Write-Host "  Acum vom face push la cod + tag" -ForegroundColor White
$confirm = Read-Host "  Continuăm? (y/N)"
if ($confirm -eq "y" -or $confirm -eq "Y") {
    git push -u origin main
    git push origin v3.5.12
    Write-Host "  Push complet!" -ForegroundColor Green
} else {
    Write-Host "  Push omis. Rulează manual mai târziu:" -ForegroundColor Yellow
    Write-Host "    git push -u origin main" -ForegroundColor Yellow
    Write-Host "    git push origin v3.5.12" -ForegroundColor Yellow
}

# Sumar
Write-Host ""
Write-Host "════════════════════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host "  GATA! Repo NeuroGraph v3.5.12 e public pe GitHub" -ForegroundColor Green
Write-Host "════════════════════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Următorii pași:" -ForegroundColor White
Write-Host "  1. Mergi pe GitHub repo → Settings" -ForegroundColor White
Write-Host "  2. About → Add description: 'Adaptive Neural Gossip Protocol'" -ForegroundColor White
Write-Host "  3. Add topics: rust, blockchain, cryptocurrency, neural-network, dag, consensus, byzantine, ed25519" -ForegroundColor White
Write-Host "  4. Settings → Features → activează Discussions + Projects + Wiki" -ForegroundColor White
Write-Host "  5. Settings → Pages → Source: main / docs → Save (pentru website)" -ForegroundColor White
Write-Host "  6. Creează GitHub Release: Releases → Draft new release → Choose tag v3.5.12" -ForegroundColor White
Write-Host "     Title: 'v3.5.12 — honest0 lives'" -ForegroundColor White
Write-Host "     Description: vezi release_notes.md" -ForegroundColor White
Write-Host ""
