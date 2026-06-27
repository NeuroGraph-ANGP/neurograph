#[derive(Clone, PartialEq, Debug)]
pub enum AttackType {
    Honest,
    Coordinated,
    RandomNoise,
    GaussianNoise,
    FlipFlop,
    Sleeper,
    Drift,
    OutlierBurst,
    Adaptive,
    Clone,
    Sybil,
}

impl AttackType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "honest" => AttackType::Honest,
            "coordinated" => AttackType::Coordinated,
            "random" => AttackType::RandomNoise,
            "gaussian" => AttackType::GaussianNoise,
            "flipflop" => AttackType::FlipFlop,
            "sleeper" => AttackType::Sleeper,
            "drift" => AttackType::Drift,
            "outlier" => AttackType::OutlierBurst,
            "adaptive" => AttackType::Adaptive,
            "clone" => AttackType::Clone,
            "sybil" => AttackType::Sybil,
            _ => {
                eprintln!("WARNING: Unknown attack type '{}', defaulting to Honest", s);
                AttackType::Honest
            }
        }
    }
}
