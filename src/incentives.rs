use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::ledger::Ledger;
use crate::state::StateManager;
use crate::config::{EPOCH_LENGTH, EPOCH_DISTRIBUTION_RATIO, EPOCH_REWARD_MIN_REP};

/// Pool-ul de comisioane (Etapa 5).
/// La fiecare EPOCH_LENGTH pași, se distribuie EPOCH_DISTRIBUTION_RATIO din pool
/// către nodurile cu reputație > EPOCH_REWARD_MIN_REP, proporțional cu numărul de
/// tranzacții pe care le-au propus și au fost finalizate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeePool {
    pub total_collected: u64,         // total fee-uri colectate vreodată
    pub current_pool: u64,            // pool curent (nedistribuit)
    pub last_epoch_step: u64,         // pasul la care s-a făcut ultima distribuire
    pub distributed_per_epoch: Vec<(u64, u64)>, // (step, amount_distributed)
}

impl FeePool {
    pub fn new() -> Self {
        FeePool {
            total_collected: 0,
            current_pool: 0,
            last_epoch_step: 0,
            distributed_per_epoch: Vec::new(),
        }
    }

    /// Adaugă fee-uri în pool (apelat când o tranzacție e finalizată).
    pub fn collect(&mut self, amount: u64) {
        self.total_collected = self.total_collected.saturating_add(amount);
        self.current_pool = self.current_pool.saturating_add(amount);
    }

    /// Verifică dacă e timpul pentru distribuirea epocală.
    pub fn should_distribute(&self, current_step: u64) -> bool {
        current_step > 0 && current_step >= self.last_epoch_step + EPOCH_LENGTH
    }

    /// Distribuie recompensele epocale către nodurile eligibile.
    /// Returnează: (suma distribuită, mapping node → amount).
    ///
    /// NOTĂ: Am separat calculul de aplicare pentru a evita conflicte de borrow
    /// la call site (ledger e împrumutat imutabil, state mutabil).
    pub fn distribute_epoch_rewards(
        &mut self,
        current_step: u64,
        reputations: &HashMap<String, f64>,
        ledger: &Ledger,
        state: &mut StateManager,
    ) -> (u64, HashMap<String, u64>) {
        let (amount, map) = self.compute_epoch_rewards(current_step, reputations, ledger);
        if amount > 0 && !map.is_empty() {
            for (node, share) in &map {
                state.apply_reward(node, *share);
            }
            self.current_pool = self.current_pool.saturating_sub(amount);
            self.distributed_per_epoch.push((current_step, amount));
        }
        (amount, map)
    }

    /// Calculează (fără a aplica) recompensele epocale.
    /// Pas separat pentru a putea fi folosit independent de StateManager.
    pub fn compute_epoch_rewards(
        &mut self,
        current_step: u64,
        reputations: &HashMap<String, f64>,
        ledger: &Ledger,
    ) -> (u64, HashMap<String, u64>) {
        if !self.should_distribute(current_step) {
            return (0, HashMap::new());
        }
        self.last_epoch_step = current_step;

        let to_distribute = (self.current_pool as f64 * EPOCH_DISTRIBUTION_RATIO) as u64;
        if to_distribute == 0 {
            return (0, HashMap::new());
        }

        let counts = ledger.finalized_count_per_node();

        let mut eligible: Vec<(String, u64, f64)> = counts.iter()
            .filter_map(|(node, count)| {
                let rep = reputations.get(node).copied().unwrap_or(0.0);
                if rep >= EPOCH_REWARD_MIN_REP && *count > 0 {
                    Some((node.clone(), *count as u64, rep))
                } else {
                    None
                }
            })
            .collect();

        if eligible.is_empty() {
            return (0, HashMap::new());
        }

        eligible.sort_by(|a, b| b.1.cmp(&a.1));

        let total_count: u64 = eligible.iter().map(|(_, c, _)| *c).sum();
        if total_count == 0 {
            return (0, HashMap::new());
        }

        let mut distributed: HashMap<String, u64> = HashMap::new();
        let mut actually_distributed = 0u64;
        for (node, count, _) in &eligible {
            let share = (to_distribute * count) / total_count;
            if share > 0 {
                distributed.insert(node.clone(), share);
                actually_distributed += share;
            }
        }

        (actually_distributed, distributed)
    }

    /// Slashing: aplică o penalizare economică unui nod.
    /// În v1.0, doar scade soldul cu SLASHING_RATIO din balanță.
    /// În viitor, cu staking, va arde stake-ul.
    pub fn slash_node(&self, node: &str, state: &mut StateManager, ratio: f64) {
        state.slash(node, ratio);
    }

    pub fn pool_balance(&self) -> u64 {
        self.current_pool
    }
}

impl Default for FeePool {
    fn default() -> Self { Self::new() }
}
