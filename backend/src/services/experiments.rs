use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Feature flag configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlag {
    pub name: String,
    pub enabled: bool,
    pub description: String,
    #[serde(default)]
    pub rollout_percentage: f32, // 0.0 to 100.0
}

/// Experiment variant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentVariant {
    pub name: String,
    pub weight: u32, // Relative weight for assignment
    pub config: HashMap<String, serde_json::Value>,
}

/// Experiment configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experiment {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub variants: Vec<ExperimentVariant>,
    #[serde(default)]
    pub traffic_percentage: f32, // 0.0 to 100.0
}

/// A/B testing and feature flag service
pub struct ExperimentService {
    flags: Arc<RwLock<HashMap<String, FeatureFlag>>>,
    experiments: Arc<RwLock<HashMap<String, Experiment>>>,
    user_assignments: Arc<RwLock<HashMap<(Uuid, String), String>>>, // (user_id, experiment_name) -> variant
}

impl ExperimentService {
    /// Create a new experiment service
    pub fn new() -> Self {
        Self {
            flags: Arc::new(RwLock::new(HashMap::new())),
            experiments: Arc::new(RwLock::new(HashMap::new())),
            user_assignments: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if a feature flag is enabled for a user
    pub async fn is_feature_enabled(&self, flag_name: &str, user_id: Uuid) -> bool {
        let flags = self.flags.read().await;

        if let Some(flag) = flags.get(flag_name) {
            if !flag.enabled {
                return false;
            }

            // Check rollout percentage
            if flag.rollout_percentage < 100.0 {
                let hash = self.hash_user_feature(user_id, flag_name);
                let user_percentage = (hash % 10000) as f32 / 100.0;
                return user_percentage < flag.rollout_percentage;
            }

            true
        } else {
            false
        }
    }

    /// Get experiment variant for a user
    pub async fn get_variant(&self, experiment_name: &str, user_id: Uuid) -> Option<String> {
        // Check if user already has an assignment
        {
            let assignments = self.user_assignments.read().await;
            if let Some(variant) = assignments.get(&(user_id, experiment_name.to_string())) {
                return Some(variant.clone());
            }
        }

        // Get experiment
        let experiments = self.experiments.read().await;
        let experiment = experiments.get(experiment_name)?;

        if !experiment.enabled {
            return None;
        }

        // Check if user is in experiment traffic
        if experiment.traffic_percentage < 100.0 {
            let hash = self.hash_user_feature(user_id, experiment_name);
            let user_percentage = (hash % 10000) as f32 / 100.0;
            if user_percentage >= experiment.traffic_percentage {
                return None;
            }
        }

        // Assign variant based on weights
        let variant_name = self.assign_variant(user_id, &experiment.variants);

        // Store assignment
        {
            let mut assignments = self.user_assignments.write().await;
            assignments.insert((user_id, experiment_name.to_string()), variant_name.clone());
        }

        // Record metric
        metrics::counter!(
            "experiment_assignments_total",
            "experiment" => experiment_name.to_string(),
            "variant" => variant_name.clone(),
        )
        .increment(1);

        Some(variant_name)
    }

    /// Get experiment variant configuration
    pub async fn get_variant_config(
        &self,
        experiment_name: &str,
        variant_name: &str,
    ) -> Option<HashMap<String, serde_json::Value>> {
        let experiments = self.experiments.read().await;
        let experiment = experiments.get(experiment_name)?;

        experiment
            .variants
            .iter()
            .find(|v| v.name == variant_name)
            .map(|v| v.config.clone())
    }

    /// Register a feature flag
    pub async fn register_flag(&self, flag: FeatureFlag) {
        let mut flags = self.flags.write().await;
        flags.insert(flag.name.clone(), flag);
    }

    /// Register an experiment
    pub async fn register_experiment(&self, experiment: Experiment) {
        let mut experiments = self.experiments.write().await;
        experiments.insert(experiment.name.clone(), experiment);
    }

    /// Update feature flag
    pub async fn update_flag(&self, flag_name: &str, enabled: bool, rollout: Option<f32>) {
        let mut flags = self.flags.write().await;
        if let Some(flag) = flags.get_mut(flag_name) {
            flag.enabled = enabled;
            if let Some(r) = rollout {
                flag.rollout_percentage = r;
            }
        }
    }

    /// List all feature flags
    pub async fn list_flags(&self) -> Vec<FeatureFlag> {
        let flags = self.flags.read().await;
        flags.values().cloned().collect()
    }

    /// List all experiments
    pub async fn list_experiments(&self) -> Vec<Experiment> {
        let experiments = self.experiments.read().await;
        experiments.values().cloned().collect()
    }

    /// Assign variant based on consistent hashing
    fn assign_variant(&self, user_id: Uuid, variants: &[ExperimentVariant]) -> String {
        if variants.is_empty() {
            return "control".to_string();
        }

        if variants.len() == 1 {
            return variants[0].name.clone();
        }

        // Calculate total weight
        let total_weight: u32 = variants.iter().map(|v| v.weight).sum();

        // Hash user ID to get a deterministic value
        let hash = self.hash_user_id(user_id);
        let bucket = (hash % total_weight as u64) as u32;

        // Find variant based on bucket
        let mut cumulative = 0u32;
        for variant in variants {
            cumulative += variant.weight;
            if bucket < cumulative {
                return variant.name.clone();
            }
        }

        // Fallback (should never happen)
        variants[0].name.clone()
    }

    /// Hash user ID for consistent assignment
    fn hash_user_id(&self, user_id: Uuid) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        user_id.hash(&mut hasher);
        hasher.finish()
    }

    /// Hash user + feature for rollout
    fn hash_user_feature(&self, user_id: Uuid, feature: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        user_id.hash(&mut hasher);
        feature.hash(&mut hasher);
        hasher.finish()
    }
}

impl Default for ExperimentService {
    fn default() -> Self {
        Self::new()
    }
}

/// Initialize default experiments and feature flags
pub async fn init_default_experiments(service: &ExperimentService) {
    // Example feature flag: New dashboard UI
    service
        .register_flag(FeatureFlag {
            name: "new_dashboard_ui".to_string(),
            enabled: true,
            description: "Enable new dashboard UI".to_string(),
            rollout_percentage: 10.0, // 10% rollout
        })
        .await;

    // Example experiment: PII detection algorithm
    service
        .register_experiment(Experiment {
            name: "pii_detection_algorithm".to_string(),
            description: "Test new PII detection algorithm".to_string(),
            enabled: true,
            traffic_percentage: 50.0, // 50% of users in experiment
            variants: vec![
                ExperimentVariant {
                    name: "control".to_string(),
                    weight: 50,
                    config: HashMap::from([("algorithm".into(), "v1".into())]),
                },
                ExperimentVariant {
                    name: "treatment".to_string(),
                    weight: 50,
                    config: HashMap::from([("algorithm".into(), "v2".into())]),
                },
            ],
        })
        .await;

    // Example experiment: Scan frequency
    service
        .register_experiment(Experiment {
            name: "scan_frequency".to_string(),
            description: "Test different scan frequencies".to_string(),
            enabled: true,
            traffic_percentage: 100.0,
            variants: vec![
                ExperimentVariant {
                    name: "daily".to_string(),
                    weight: 33,
                    config: HashMap::from([("frequency_days".into(), 1.into())]),
                },
                ExperimentVariant {
                    name: "weekly".to_string(),
                    weight: 34,
                    config: HashMap::from([("frequency_days".into(), 7.into())]),
                },
                ExperimentVariant {
                    name: "monthly".to_string(),
                    weight: 33,
                    config: HashMap::from([("frequency_days".into(), 30.into())]),
                },
            ],
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_feature_flag_rollout() {
        let service = ExperimentService::new();

        service
            .register_flag(FeatureFlag {
                name: "test_flag".to_string(),
                enabled: true,
                description: "Test flag".to_string(),
                rollout_percentage: 50.0,
            })
            .await;

        // Test multiple users
        let mut enabled_count = 0;
        for i in 0..100 {
            let user_id = Uuid::from_u128(i);
            if service.is_feature_enabled("test_flag", user_id).await {
                enabled_count += 1;
            }
        }

        // Should be roughly 50% (allow 20% variance)
        assert!(enabled_count >= 30 && enabled_count <= 70);
    }

    #[tokio::test]
    async fn test_experiment_variant_assignment() {
        let service = ExperimentService::new();

        service
            .register_experiment(Experiment {
                name: "test_exp".to_string(),
                description: "Test".to_string(),
                enabled: true,
                traffic_percentage: 100.0,
                variants: vec![
                    ExperimentVariant {
                        name: "control".to_string(),
                        weight: 50,
                        config: HashMap::new(),
                    },
                    ExperimentVariant {
                        name: "treatment".to_string(),
                        weight: 50,
                        config: HashMap::new(),
                    },
                ],
            })
            .await;

        let user_id = Uuid::new_v4();
        let variant1 = service.get_variant("test_exp", user_id).await;
        let variant2 = service.get_variant("test_exp", user_id).await;

        // Should be consistent
        assert_eq!(variant1, variant2);
        assert!(variant1.is_some());
    }
}
