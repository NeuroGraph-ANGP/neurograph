use ndarray::Array1;
use std::collections::HashMap;

/// DAG adaptiv cu învățare Hebbiană (folosit pentru predicții interne).
/// Păstrat pentru compatibilitate — Etapa 0 mută consensul pe DagProposalMessage.
pub struct AdaptiveDag {
    pub order: Vec<String>,
    nodes: HashMap<String, Array1<f64>>,
    parents: HashMap<String, Vec<String>>,
    weights: HashMap<String, Vec<f64>>,
}

impl AdaptiveDag {
    pub fn new() -> Self {
        Self {
            order: Vec::new(),
            nodes: HashMap::new(),
            parents: HashMap::new(),
            weights: HashMap::new(),
        }
    }

    pub fn create_node(&mut self, parents: &[String], vote: Array1<f64>) -> String {
        let id = format!("node_{}", self.order.len());
        self.order.push(id.clone());
        self.nodes.insert(id.clone(), vote);
        if !parents.is_empty() {
            let parent_vec = parents.to_vec();
            let initial_weight = 1.0 / parent_vec.len() as f64;
            self.parents.insert(id.clone(), parent_vec);
            self.weights.insert(id.clone(), vec![initial_weight; parents.len()]);
        }
        id
    }

    pub fn update(&mut self, node_id: &str, vote: Array1<f64>) {
        self.update_with_alpha(node_id, vote, crate::config::ALPHA);
    }

    /// ════════════════════════════════════════════════════════════════
    /// ÎNVĂȚARE HEBBIANĂ CU α ADAPTIV (v2.4 #5)
    /// ════════════════════════════════════════════════════════════════
    ///
    /// α adaptiv creează feedback loop:
    ///   - agreement mare (predicție ≈ consens): α mare → consolidare rapidă
    ///     → nodul onest își întărește predicțiile corecte
    ///   - agreement mic (predicție ≠ consens): α mic → consolidare lentă
    ///     → atacatorul nu-și poate consolida predicțiile perturbate
    ///
    /// Formula: weight[i] += α × (similarity - weight[i])
    /// similarity = 1 / (1 + dist(vote, parent[i]))
    pub fn update_with_alpha(&mut self, node_id: &str, vote: Array1<f64>, alpha: f64) {
        if let Some(parent_list) = self.parents.get(node_id) {
            if let Some(w) = self.weights.get_mut(node_id) {
                for (i, parent_id) in parent_list.iter().enumerate() {
                    if let Some(parent_val) = self.nodes.get(parent_id) {
                        let diff = &vote - parent_val;
                        let dist = diff.dot(&diff).sqrt();
                        let similarity = 1.0 / (1.0 + dist);
                        let current_weight = w.get(i).copied().unwrap_or(1.0);
                        // α adaptiv controlează viteza de învățare
                        w[i] = current_weight + alpha * (similarity - current_weight);
                        if w[i] < 0.01 { w[i] = 0.01; }
                    }
                }
            }
        }
        if let Some(existing) = self.nodes.get_mut(node_id) {
            *existing = vote;
        }
    }

    pub fn predict(&self, node_id: &str) -> Option<Array1<f64>> {
        let dim = self.nodes.get(node_id)?.len();
        if let Some(parent_list) = self.parents.get(node_id) {
            if !parent_list.is_empty() {
                if let Some(w) = self.weights.get(node_id) {
                    let mut prediction = Array1::zeros(dim);
                    let mut total_weight = 0.0;
                    for (i, parent_id) in parent_list.iter().enumerate() {
                        if let Some(parent_val) = self.nodes.get(parent_id) {
                            let weight = w.get(i).copied().unwrap_or(1.0);
                            prediction = prediction + parent_val * weight;
                            total_weight += weight;
                        }
                    }
                    if total_weight > 0.0 {
                        return Some(prediction / total_weight);
                    }
                }
            }
        }
        self.nodes.get(node_id).cloned()
    }

    pub fn last_node_id(&self) -> Option<String> {
        self.order.last().cloned()
    }

    /// v3.5.11: Pruning — elimină nodurile vechi, păstrează doar ultimele `keep`.
    ///
    /// IMPORTANT: Acest pruning NU afectează securitatea pentru că:
    ///   1. Attack detection folosește `received_proposals` (din AngpNode), nu AdaptiveDag
    ///   2. Cluster capping folosește propuneri curente, nu istoric AdaptiveDag
    ///   3. Reputația EMA folosește erori curente, nu istoric AdaptiveDag
    ///   4. AdaptiveDag doar predice — predicția se bazează pe parent direct (ultimul nod)
    ///
    /// Păstrăm 50 de noduri (suficient pentru Hebbian learning + momentum).
    pub fn prune(&mut self, keep: usize) {
        if self.order.len() <= keep { return; }
        let to_remove = self.order.len() - keep;
        for _ in 0..to_remove {
            if let Some(old_id) = self.order.first().cloned() {
                self.order.remove(0);
                self.nodes.remove(&old_id);
                self.parents.remove(&old_id);
                self.weights.remove(&old_id);
            }
        }
    }
}

impl Default for AdaptiveDag {
    fn default() -> Self { Self::new() }
}
