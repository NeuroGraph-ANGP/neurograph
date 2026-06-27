pub const DIM: usize = 4;
pub const ALPHA: f64 = 0.05;
pub const BETA: f64 = 0.002;

// ─── Bootstrapping (conform whitepaper) ─────────────────────────────
pub const BOOTSTRAP_STEPS: u64 = 150;
pub const GRACE_PERIOD: u64 = 200;
pub const WARMUP_STEPS: u64 = 0;

// Prag de filtrare pentru consens (whitepaper: 0.9)
pub const REPUTATION_FILTER_THRESHOLD: f64 = 0.9;

// Dimensiuni ferestre
pub const WINDOW_SIZE: usize = 200;
pub const NOVELTY_ALPHA: f64 = 0.1;
pub const NOVELTY_HISTORY_SIZE: usize = 50;
pub const LOW_NOVELTY_THRESHOLD: f64 = 0.0;

// PoW
pub const POW_DIFFICULTY: usize = 4;

// ─── Pasul 3: Double-Spend Detection ────────────────────────────────
pub const NONCE_HISTORY_MAX_PER_SENDER: usize = 1024;

// ─── Pasul 5: Finality cu Threshold Dinamic ─────────────────────────
pub const EXPECTED_ACTIVE_NODES: usize = 10;
pub const FINALITY_BASE_THRESHOLD: f64 = 0.5;
pub const FINALITY_MIN_THRESHOLD: f64 = 0.3;
pub const FINALITY_MAX_THRESHOLD: f64 = 0.9;

// ─── Pasul 4: Embedding Deterministic ───────────────────────────────
pub const EMBED_WINDOW_BYTES: usize = 8;

// ═════════════════════════════════════════════════════════════════════
// NeuroGraph v2.0 — Etapele 0-5
// ═════════════════════════════════════════════════════════════════════

// ─── Etapa 0: DagProposalMessage ────────────────────────────────────
pub const PROPOSAL_TIPS_COUNT: usize = 3;
/// v3.2: Redus de la 0.5 la 0.15. State_root poate diferi legitim
/// în timpul sync-ului — 0.5 per pas provoca death spiral.
pub const STATE_ROOT_PENALTY: f64 = 0.15;
/// v3.2: Penalty per tip diferit (normalizat în cod după numărul de tips).
pub const TIP_DIFF_PENALTY: f64 = 0.05;
/// v3.2: Nu mai folosim filtru rep>0.9 — folosim ALL proposals cu weighted median.
pub const MIN_CONSENSUS_NODES: usize = 1;

// ─── v3.2: Attack Detection ────────────────────────────────────────
/// Clone detection: dacă predicția unui nod e în CLONE_EPSILON de predicția
/// altui nod la același pas, e marcat ca potential clone.
pub const CLONE_EPSILON: f64 = 0.01;
/// Clone detection: câte pași consecutivi de clonare înainte de penalizare.
pub const CLONE_STREAK_THRESHOLD: u32 = 5;
/// Penalty aplicat clone-urilor (multiplicator pe eroare).
pub const CLONE_PENALTY_MULTIPLIER: f64 = 3.0;

/// Coordination detection: dacă N noduri au predicții în COORDINATION_EPSILON
/// unele de altele, sunt marcate ca coordonate (pentru penalty).
pub const COORDINATION_EPSILON: f64 = 0.01;
/// v3.4: Epsilon separat pentru cluster weight capping în compute_consensus.
/// Mai STRICT — doar predicții EXACT identice (L2 < 0.003) se grupează.
/// Honest nodes cu EMA-smoothed noise (L2 ~ 0.006) NU sunt afectate.
pub const CLUSTER_CAPPING_EPSILON: f64 = 0.003;
/// Numărul minim de noduri într-un cluster pentru a fi considerat coordonat.
pub const COORDINATION_MIN_CLUSTER: usize = 3;
/// Penalty aplicat cluster-elor coordonate.
pub const COORDINATION_PENALTY: f64 = 0.5;

/// Adaptive detection: dacă un nod are erori mari DOAR când |consens-0.5| < ADAPTIVE_ZONE
/// și erori mici în rest, e un atacator adaptive.
pub const ADAPTIVE_ZONE: f64 = 0.15;
/// Numărul de pași pentru a detecta pattern-ul adaptive.
pub const ADAPTIVE_DETECTION_WINDOW: usize = 100;
/// Penalty aplicat atacatorilor adaptive.
pub const ADAPTIVE_PENALTY: f64 = 0.5;
/// v3.3: Praguri adaptive detector ajustate pentru soft error (tanh cap-ează la ~0.3)
pub const ADAPTIVE_ERR_HIGH: f64 = 0.15;
pub const ADAPTIVE_ERR_LOW: f64 = 0.05;

// ─── v3.4: Cluster-aware Weight Capping ────────────────────────────
/// Când un cluster de N noduri are predicții identice (în COORDINATION_EPSILON),
/// weight-ul total al cluster-ului e cap-at la:
///   min(N × individual_weight, total_weight × CLUSTER_WEIGHT_CAP_RATIO)
///
/// Asta previne ca un cluster coordonat (chiar cu 47% atacatori) să domine
/// weighted median-ul. Honest nodes (cu zgomot natural) nu se grupează
/// în clustere identice, deci nu sunt afectate.
pub const CLUSTER_WEIGHT_CAP_RATIO: f64 = 0.30;

/// v3.4: Prediction Jump Detector
/// Detectează atacatori adaptive care comută brusc între mod honest și atac.
/// Dacă predicția unui nod sare cu mai mult de JUMP_THRESHOLD între pași
/// consecutivi, e marcat ca potential adaptive.
pub const PREDICTION_JUMP_THRESHOLD: f64 = 0.2;
pub const PREDICTION_JUMP_STREAK: u32 = 3;
pub const PREDICTION_JUMP_PENALTY: f64 = 0.6;

/// v3.4: Behavioral decay — nodes care își schimbă brusc comportamentul
/// (varianță mare în predicții) primesc decay suplimentar pe reputație.
pub const BEHAVIORAL_DECAY_THRESHOLD: f64 = 0.15;
pub const BEHAVIORAL_DECAY_FACTOR: f64 = 0.95;

// ─── v3.3: Honest Node Protection ──────────────────────────────────
/// Reputation floor: dacă un nod a fost deasupra REPUTATION_FLOOR_THRESHOLD
/// pentru REPUTATION_FLOOR_MIN_STEPS pași, reputația sa nu poate scădea
/// sub REPUTATION_FLOOR_VALUE pentru următorii REPUTATION_FLOOR_GRACE pași.
/// Protejează honest nodes de death spiral temporar.
pub const REPUTATION_FLOOR_THRESHOLD: f64 = 0.3;
pub const REPUTATION_FLOOR_MIN_STEPS: usize = 20;
pub const REPUTATION_FLOOR_VALUE: f64 = 0.3;
pub const REPUTATION_FLOOR_GRACE: u64 = 500;

/// Soft error function: raw L2 e înlocuit cu tanh(L2 / SOFT_ERROR_SCALE).
/// Cap-ează eroarea maximă la 1.0, prevenind ca un singur pas de dezacord
/// să distrugă reputația unui honest node.
pub const SOFT_ERROR_SCALE: f64 = 0.2;

/// Adaptive error normalization: împarte eroarea neurală la mediana
/// distanțelor pairwise dintre toate predicțiile. Honest nodes (care sunt
/// natural apropiate) vor avea eroare normalizată mică.
pub const ERROR_NORMALIZATION: bool = true;

// ─── v3.3: Temporal Consistency Detector ───────────────────────────
/// Măsoară variația predicției pas-la-pas. Honest nodes (EMA + sinusoidă)
/// au variație mică; GaussianNoise/RandomNoise/Adaptive au variație mare.
pub const TEMPORAL_WINDOW: usize = 20;
/// Dacă variația medie (L2 între pași consecutivi) depășește acest prag,
/// nodul e marcat ca temporal-inconsistent (atacator noise).
pub const TEMPORAL_INCONSISTENCY_THRESHOLD: f64 = 0.08;
/// Penalty aplicat pentru temporal inconsistency.
pub const TEMPORAL_PENALTY: f64 = 0.8;

// ─── Etapa 1: Finalizare în lot + Fee ───────────────────────────────
/// La fiecare FINALIZATION_INTERVAL pași, încercăm să finalizăm un batch.
/// v3.5.5: Redus de la 10 la 5 pentru finality mai rapid (50ms în loc de 500ms cu LOOP_SLEEP_MS=10).
pub const FINALIZATION_INTERVAL: u64 = 5;
/// Pragul de aprobare (70%) — tx trebuie să fie propusă de ≥70% din nodurile oneste active.
pub const BATCH_APPROVAL_THRESHOLD: f64 = 0.7;
/// Fee-ul implicit per tranzacție, în milliANGP (1 ANGP = 1000 milliANGP).
pub const DEFAULT_TX_FEE_MILLI: u64 = 1;  // 0.001 ANGP

// ─── Etapa 2: Weighted Random Walk (MCMC) ──────────────────────────
/// Numărul de pași în random walk-ul invers (de la tips spre genesis).
pub const MCMC_WALK_LENGTH: usize = 15;
/// Inversează ponderea (true = preferă noduri mai ușoare, false = preferă mai grele).
pub const MCMC_PREFER_LIGHT: bool = true;
/// Influența reputației în selecție (0 = ignoră, 1 = doar reputație).
pub const MCMC_REP_INFLUENCE: f64 = 0.3;

// ─── Etapa 3: State Manager ─────────────────────────────────────────
/// Soldul inițial pentru conturi noi (pentru demo; în producție: genesis allocation).
pub const GENESIS_BALANCE_MILLI: u64 = 1_000_000; // 1000 ANGP

// ─── Etapa 5: Tokenomics ────────────────────────────────────────────
/// La fiecare EPOCH pași, se distribuie recompense epocale.
pub const EPOCH_LENGTH: u64 = 1000;
/// Proporția din fee-pool distribuită per epoch (restul rămâne în pool).
pub const EPOCH_DISTRIBUTION_RATIO: f64 = 0.5;
/// Reputația minimă pentru a primi recompense epocale.
pub const EPOCH_REWARD_MIN_REP: f64 = 0.95;
/// slashing ratio pentru propuneri conflictuale (dacă e activat).
pub const SLASHING_RATIO: f64 = 0.1;

// ─── Tuning semnal onest (păstrat pentru compatibilitate v1.1) ─────
/// NOTĂ: În NeuroGraph v2.0, consensul NU mai folosește semnalul vectorial.
/// Aceste constante rămân doar pentru `real_signal()` care e încă definit
/// în node.rs pentru backward compatibility / testare.
pub const SIGNAL_TIME_SCALE: f64 = 0.05;
pub const SIGNAL_NOISE_STD: f64 = 0.005;
pub const SIGNAL_AMPLITUDE: f64 = 0.4;
pub const SIGNAL_BASELINE: f64 = 0.5;
pub const SIGNAL_PHASE_OFFSET: f64 = 0.1;
pub const SIGNAL_EMA_ALPHA: f64 = 0.3;

// ─── v2.2: Conflict Vote (Etapa 1 completion) ──────────────────────
/// Fereastra de vot pentru conflicte (în pași). După expirare, conflictele
/// sunt rezolvate prin vot ponderat de reputație.
pub const CONFLICT_VOTE_WINDOW: u64 = 20;

// ─── v2.2: Rate Limiting ───────────────────────────────────────────
/// Pragul de mesaje dropped peste care un peer e penalizat în reputație.
pub const RATE_LIMIT_PENALTY_THRESHOLD: u64 = 10;
/// Penalizare aplicată reputației pentru flooding.
pub const RATE_LIMIT_PENALTY: f64 = 0.05;

// ════════════════════════════════════════════════════════════════════
// v2.4 — Adaptive Learning Rate (#5) + Predictive Tip Selection (#3)
// ════════════════════════════════════════════════════════════════════
//
// Ambele mecanisme creează un feedback loop pozitiv pentru honest nodes
// și negativ pentru atacatori:
//   - Honest: predicții corecte → α mare → consolidare rapidă → tips comune
//   - Attacker: predicții greșite → α mic → consolidare lentă → tips neobișnuite
//                → reputație ↓ → α și mai mic → loop negativ

// ─── #5: Adaptive Learning Rate ────────────────────────────────────
/// α de bază pentru învățarea Hebbiană (când agreement e maxim).
pub const HEBBIAN_BASE_ALPHA: f64 = 0.10;
/// α minim (când agreement e 0 — dezacord total cu consensul).
pub const HEBBIAN_MIN_ALPHA: f64 = 0.005;
/// Puterea cu care agreement-ul modifică α.
/// α = MIN + (BASE - MIN) × agreement^POWER
/// POWER > 1 face curba mai abruptă (mai punitivă pentru dezacord).
pub const HEBBIAN_ALPHA_AGREEMENT_POWER: f64 = 2.0;
/// Numărul normalizator pentru L2 distance → agreement.
/// agreement = max(0, 1 - L2(pred, median) / NORM)
pub const HEBBIAN_AGREEMENT_NORM: f64 = 1.0;

// ─── #3: Predictive Tip Selection ──────────────────────────────────
/// Pondere propria predicție în blend (predicție vs median consens).
/// blend = OWN_WEIGHT × own_pred + (1 - OWN_WEIGHT) × consensus_median
/// OWN_WEIGHT mare = node își urmează propria predicție (mai independent)
/// OWN_WEIGHT mic = node se aliniază cu consensul (mai cooperant)
pub const PREDICTIVE_OWN_WEIGHT: f64 = 0.4;
/// Pondere momentum pentru tips istoric comune.
/// Cu cât un tip a fost comun mai des în istoric, cu atât mai probabil
/// să fie selectat din nou (pattern reinforcement).
pub const PREDICTIVE_MOMENTUM_WEIGHT: f64 = 0.3;
/// Câte trecute comun-uri de tips păstrăm în istoric (sliding window).
pub const PREDICTIVE_MOMENTUM_HISTORY: usize = 10;

// ════════════════════════════════════════════════════════════════════
// v2.5 — Scalability & Speed (target: 500K TPS)
// ════════════════════════════════════════════════════════════════════

/// v2.5: Interval flush pentru state/mempool/ledger (în pași).
/// Înainte: save_to_disk la fiecare operație (500K disk I/Os/s la 500K TPS).
/// Acum: flush o dată la 100 pași (~1 secundă la 10ms/step).
pub const FLUSH_INTERVAL_STEPS: u64 = 100;

/// v2.5: Mărimea maximă a unui batch de tranzacții pentru network batching.
/// La 500K TPS cu loop de 10ms: 5000 txs/step.
/// Le grupăm în mesaje de MAX_TX_BATCH_SIZE pentru a reduce overhead-ul de rețea.
pub const MAX_TX_BATCH_SIZE: usize = 256;

/// v3.5.5: Sleep între pași (era 50ms în v2.4, 10ms în v2.5).
/// 10ms = 100 pași/sec → 5× mai multe propuneri/sec.
/// Mărim la 10ms pentru TPS mai mare pe Windows.
pub const LOOP_SLEEP_MS: u64 = 10;

// ════════════════════════════════════════════════════════════════════
// v3.0 — Sharding (target: 1M TPS pe sistem)
// ════════════════════════════════════════════════════════════════════
//
// Sharding account-based (ca Near/Avalanche):
//   - 16 shards × 63K TPS/shard = 1.008M TPS pe sistem
//   - shard_id(addr) = SHA-512/256(addr) % N_SHARDS (determinist)
//   - Fiecare nod procesează un subset de shards (--shards 0,1,2)
//   - Cross-shard tx = 2 faze:
//       Phase 1: LockTx în shard-ul sender (debitează, emite receipt)
//       Phase 2: CommitTx în shard-ul receiver (creditează, consumă receipt)
//   - Reputația rămâne GLOBALĂ (un nod e evaluat pe toate propunerile sale)
//   - AdaptiveDag Hebbian rămâne pe shard-ul procesat
//   - Consens emergent (mediană ponderată) rămâne pe shard-ul procesat
//
// Importante:
//   - PROTOCOLUL ANGP nu se schimbă — DagProposal rămâne aceeași structură
//   - SECURITATEA rămâne — semnături Ed25519, SHA-512/256, double-spend O(1)
//   - FĂRĂ staking — minerit onest rămâne mecanismul
//
// Routing:
//   - Tx intra-shard (sender și receiver în același shard): procesat local
//   - Tx cross-shard: split în LockTx + CommitTx, fiecare în shard-ul corect

/// Numărul total de shards în sistem.
/// v3.5.7: Crescut de la 16 la 961 pentru a atinge 1M TPS target.
/// Calcul: TPS_sistem = N_SHARDS × TPS_nod × (1 - cross_shard_overhead)
///         1,000,000  = 961 × 1,300 × 0.8 = 999,440 TPS (cu batch verify nativ)
///         Cu TPS_nod actual (130): 961 × 130 × 0.8 = ~100,000 TPS (limita fara batch verify)
///
/// IMPORTANT: 961 = 31^2 (pătrat perfect) — facilitează topologia 2D grid pentru
/// cross-shard communication (shard_id = x*31 + y, vecini N/S/E/W).
///
/// Pentru a funcționa corect cu BFT 70%, necesită minim 3 noduri/shard:
///   961 shards × 3 noduri/shard = 2,883 noduri minim (pentru redundanță)
///   961 shards × 4 noduri/shard = 3,844 noduri (recomandat pentru BFT sigur)
pub const N_SHARDS: u32 = 961;

/// Default shards pentru un nod (dacă --shards nu e specificat).
/// Empty = procesează toate shards (compatibility mode, ca v2.5).
pub const DEFAULT_SHARDS: &[u32] = &[];

/// Cross-shard receipt expiry (în pași). Dacă un receipt nu e consumat
/// în atâția pași, e returnat sender-ului (refund).
pub const CROSS_SHARD_RECEIPT_EXPIRY: u64 = 1000;


