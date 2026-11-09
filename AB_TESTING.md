# A/B Testing & Feature Flags

VectorScrub includes a built-in experimentation framework for A/B testing and feature flags.

## Overview

The experimentation system provides:

- **Feature Flags:** On/off toggles with gradual rollout
- **A/B Testing:** Multi-variant experiments with configurable traffic
- **Consistent Assignment:** Users always see the same variant
- **Metrics Tracking:** Automatic experiment event tracking

## Architecture

### Components

1. **ExperimentService** (Rust backend)
   - Feature flag evaluation
   - Variant assignment
   - Configuration management

2. **Frontend Integration**
   - React hooks for feature flags
   - Experiment tracking
   - Analytics integration

3. **Analytics Pipeline**
   - Event collection
   - Experiment metrics
   - Statistical analysis

## Feature Flags

### Creating a Feature Flag

```rust
use crate::services::experiments::{ExperimentService, FeatureFlag};

let service = ExperimentService::new();

service.register_flag(FeatureFlag {
    name: "new_dashboard_ui".to_string(),
    enabled: true,
    description: "Enable new dashboard UI".to_string(),
    rollout_percentage: 10.0, // Start at 10%
}).await;
```

### Checking Feature Flags

#### Backend (Rust)

```rust
let user_id = Uuid::new_v4();

if service.is_feature_enabled("new_dashboard_ui", user_id).await {
    // Show new UI
} else {
    // Show old UI
}
```

#### Frontend (TypeScript)

```typescript
import { useFeatureFlag } from '@/hooks/useExperiments';

function Dashboard() {
  const newUIEnabled = useFeatureFlag('new_dashboard_ui');

  return newUIEnabled ? <NewDashboard /> : <OldDashboard />;
}
```

### Gradual Rollout

Feature flags support gradual rollout using consistent hashing:

```rust
// Start at 1%
service.update_flag("new_dashboard_ui", true, Some(1.0)).await;

// Increase to 10%
service.update_flag("new_dashboard_ui", true, Some(10.0)).await;

// Increase to 50%
service.update_flag("new_dashboard_ui", true, Some(50.0)).await;

// Full rollout
service.update_flag("new_dashboard_ui", true, Some(100.0)).await;
```

## A/B Testing

### Creating an Experiment

```rust
use crate::services::experiments::{Experiment, ExperimentVariant};
use std::collections::HashMap;

service.register_experiment(Experiment {
    name: "pii_detection_algorithm".to_string(),
    description: "Test new PII detection algorithm".to_string(),
    enabled: true,
    traffic_percentage: 50.0, // 50% of users in experiment
    variants: vec![
        ExperimentVariant {
            name: "control".to_string(),
            weight: 50, // 50% of experiment traffic
            config: HashMap::from([
                ("algorithm".into(), "v1".into()),
                ("threshold".into(), 0.6.into()),
            ]),
        },
        ExperimentVariant {
            name: "treatment".to_string(),
            weight: 50, // 50% of experiment traffic
            config: HashMap::from([
                ("algorithm".into(), "v2".into()),
                ("threshold".into(), 0.7.into()),
            ]),
        },
    ],
}).await;
```

### Getting Variant Assignment

#### Backend

```rust
let user_id = Uuid::new_v4();

if let Some(variant) = service.get_variant("pii_detection_algorithm", user_id).await {
    let config = service.get_variant_config("pii_detection_algorithm", &variant).await;

    // Use variant configuration
    match variant.as_str() {
        "control" => run_v1_algorithm(),
        "treatment" => run_v2_algorithm(),
        _ => {}
    }
}
```

#### Frontend

```typescript
import { useExperiment } from '@/hooks/useExperiments';

function ScanButton() {
  const { variant, config } = useExperiment('scan_button_text');

  const buttonText = variant === 'treatment'
    ? 'Start Privacy Scan'
    : 'Scan Now';

  return <button onClick={handleScan}>{buttonText}</button>;
}
```

## Example Experiments

### 1. UI Copy Testing

Test different button text:

```rust
Experiment {
    name: "scan_button_text".to_string(),
    description: "Test scan button copy".to_string(),
    enabled: true,
    traffic_percentage: 100.0,
    variants: vec![
        ExperimentVariant {
            name: "control".to_string(),
            weight: 50,
            config: HashMap::from([("text".into(), "Scan Now".into())]),
        },
        ExperimentVariant {
            name: "treatment".to_string(),
            weight: 50,
            config: HashMap::from([("text".into(), "Start Privacy Scan".into())]),
        },
    ],
}
```

### 2. Algorithm Testing

Test detection algorithms:

```rust
Experiment {
    name: "pii_detection_algorithm".to_string(),
    description: "Compare detection algorithms".to_string(),
    enabled: true,
    traffic_percentage: 50.0, // Only 50% of users
    variants: vec![
        ExperimentVariant {
            name: "control".to_string(),
            weight: 50,
            config: HashMap::from([
                ("algorithm".into(), "pattern_matching".into()),
                ("confidence_threshold".into(), 0.6.into()),
            ]),
        },
        ExperimentVariant {
            name: "treatment".to_string(),
            weight: 50,
            config: HashMap::from([
                ("algorithm".into(), "ml_model".into()),
                ("confidence_threshold".into(), 0.7.into()),
            ]),
        },
    ],
}
```

### 3. Multi-Variant Testing

Test multiple options:

```rust
Experiment {
    name: "scan_frequency".to_string(),
    description: "Optimal scan frequency".to_string(),
    enabled: true,
    traffic_percentage: 100.0,
    variants: vec![
        ExperimentVariant {
            name: "daily".to_string(),
            weight: 33,
            config: HashMap::from([("days".into(), 1.into())]),
        },
        ExperimentVariant {
            name: "weekly".to_string(),
            weight: 34,
            config: HashMap::from([("days".into(), 7.into())]),
        },
        ExperimentVariant {
            name: "monthly".to_string(),
            weight: 33,
            config: HashMap::from([("days".into(), 30.into())]),
        },
    ],
}
```

## Metrics & Analysis

### Automatic Metrics

The system automatically tracks:

```prometheus
# Variant assignments
experiment_assignments_total{experiment="scan_button_text", variant="control"} 5234
experiment_assignments_total{experiment="scan_button_text", variant="treatment"} 5189

# Feature flag evaluations
feature_flag_evaluations_total{flag="new_dashboard_ui", result="enabled"} 1234
feature_flag_evaluations_total{flag="new_dashboard_ui", result="disabled"} 8766
```

### Custom Event Tracking

Track experiment-specific metrics:

```typescript
import { trackExperimentEvent } from '@/lib/experiments';

function handleScan() {
  const { variant } = useExperiment('scan_button_text');

  // Track button click
  trackExperimentEvent('scan_button_text', variant, 'button_clicked');

  // Perform scan
  const result = await startScan();

  // Track outcome
  trackExperimentEvent('scan_button_text', variant, 'scan_completed', {
    exposures_found: result.exposures.length,
    scan_duration: result.duration,
  });
}
```

### Statistical Analysis

Calculate statistical significance:

```python
from scipy import stats

def analyze_experiment(control_data, treatment_data):
    """
    Perform statistical analysis on experiment results.

    Args:
        control_data: List of metric values for control group
        treatment_data: List of metric values for treatment group

    Returns:
        Dict with analysis results
    """
    # T-test
    t_stat, p_value = stats.ttest_ind(control_data, treatment_data)

    # Effect size (Cohen's d)
    pooled_std = np.sqrt(
        (np.std(control_data) ** 2 + np.std(treatment_data) ** 2) / 2
    )
    cohens_d = (np.mean(treatment_data) - np.mean(control_data)) / pooled_std

    return {
        'control_mean': np.mean(control_data),
        'treatment_mean': np.mean(treatment_data),
        'p_value': p_value,
        'significant': p_value < 0.05,
        'effect_size': cohens_d,
        'lift': ((np.mean(treatment_data) - np.mean(control_data))
                 / np.mean(control_data) * 100),
    }

# Example usage
control_conversions = [0, 1, 0, 1, 1, 0, ...]
treatment_conversions = [1, 1, 0, 1, 1, 1, ...]

results = analyze_experiment(control_conversions, treatment_conversions)
print(f"Treatment lifted conversion by {results['lift']:.2f}%")
print(f"P-value: {results['p_value']:.4f}")
print(f"Significant: {results['significant']}")
```

## Best Practices

### 1. Start Small

Begin with small traffic percentages:

```rust
// Start at 5% of users
traffic_percentage: 5.0
```

Gradually increase as confidence grows.

### 2. Use Meaningful Names

```rust
// Good
"new_dashboard_ui"
"pii_detection_algorithm_v2"
"scan_button_copy_test"

// Bad
"test1"
"experiment_a"
"new_thing"
```

### 3. Document Experiments

Include clear descriptions:

```rust
Experiment {
    name: "checkout_flow_simplification".to_string(),
    description: "Test simplified 2-step checkout vs current 3-step flow. \
                  Hypothesis: Fewer steps will increase conversion by 10%. \
                  Success metric: Completed subscriptions / Started checkouts".to_string(),
    // ...
}
```

### 4. Set Success Metrics Upfront

Define success criteria before starting:

```typescript
const experimentConfig = {
  name: 'scan_button_text',
  successMetrics: {
    primary: 'scan_completion_rate',
    secondary: ['button_click_rate', 'time_to_scan'],
  },
  minimumSampleSize: 1000,
  minimumDuration: '7 days',
  targetLift: 0.05, // 5% improvement
};
```

### 5. Run Until Statistical Significance

Don't stop too early:

- Minimum sample size (e.g., 1000 per variant)
- Minimum duration (e.g., 1 week to capture weekly patterns)
- p-value < 0.05 for significance

### 6. Consider Seasonality

Run experiments for full cycles:

- Weekly experiments: 1-2 weeks
- Monthly experiments: 4-6 weeks
- Account for holidays/special events

### 7. Clean Up Old Experiments

Remove completed experiments:

```rust
// After experiment is conclusive
service.update_experiment("old_experiment", false).await; // Disable

// Remove entirely
service.remove_experiment("old_experiment").await;

// Or make it a permanent feature flag
service.register_flag(FeatureFlag {
    name: "winning_variant".to_string(),
    enabled: true,
    rollout_percentage: 100.0,
}).await;
```

## Frontend Integration

### React Hooks

Create hooks for easy integration:

```typescript
// hooks/useExperiments.ts

export function useFeatureFlag(flagName: string): boolean {
  const [enabled, setEnabled] = useState(false);

  useEffect(() => {
    fetch(`/api/v1/experiments/flags/${flagName}`)
      .then(res => res.json())
      .then(data => setEnabled(data.enabled));
  }, [flagName]);

  return enabled;
}

export function useExperiment(experimentName: string) {
  const [variant, setVariant] = useState<string | null>(null);
  const [config, setConfig] = useState<any>(null);

  useEffect(() => {
    fetch(`/api/v1/experiments/${experimentName}/variant`)
      .then(res => res.json())
      .then(data => {
        setVariant(data.variant);
        setConfig(data.config);
      });
  }, [experimentName]);

  return { variant, config };
}
```

### Usage Example

```typescript
function Dashboard() {
  const newUI = useFeatureFlag('new_dashboard_ui');
  const { variant, config } = useExperiment('scan_frequency');

  if (newUI) {
    return <NewDashboard scanFrequency={config?.days} />;
  }

  return <OldDashboard />;
}
```

## Production Checklist

Before launching experiments to production:

- [ ] Define success metrics and target lift
- [ ] Set minimum sample size and duration
- [ ] Implement metrics tracking
- [ ] Configure alerts for errors
- [ ] Test both variants work correctly
- [ ] Document experiment hypothesis
- [ ] Set experiment end date
- [ ] Plan for rollout/rollback
- [ ] Stakeholder approval

## Troubleshooting

### Users Seeing Different Variants

Check that variant assignment is:
- Deterministic (same user always gets same variant)
- Consistent across sessions
- Stored server-side

### Metrics Not Tracking

Verify:
- Event tracking is implemented
- Prometheus metrics are being exported
- Analytics pipeline is running

### Statistical Significance Issues

- Increase sample size
- Run longer duration
- Check for outliers
- Verify metric is meaningful

---

**Last Updated:** 2026-11-09
