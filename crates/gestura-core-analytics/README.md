# gestura-core-analytics

Usage analytics, insights, and personalized recommendations for Gestura.

## What belongs here

- **Analytics** — event tracking, usage patterns, performance metrics, error analysis
- **Recommendations** — behaviour-based personalized suggestions, confidence scoring, feedback loop

This crate merges the `analytics` and `recommendations` domains into a
single package covering the full analytics pipeline.

## Modules

- `analytics`        — `UsageAnalytics`, `UsageEvent`, `AnalyticsInsights`, `PerformanceMetrics`
- `recommendations`  — `PersonalizedRecommendationEngine`, `Recommendation`, `UserBehaviorPattern`

## Key types

| Type | Description |
|------|-------------|
| `UsageAnalytics` | Central analytics engine (event recording, insights generation) |
| `UsageEvent` / `EventType` | Tracked usage events |
| `AnalyticsInsights` | Aggregated usage insights |
| `AnalyticsConfig` / `PrivacyMode` | Privacy-aware configuration |
| `PersonalizedRecommendationEngine` | Behaviour-driven recommendation engine |
| `Recommendation` / `RecommendationType` | Individual recommendations |
| `UserBehaviorPattern` | Learned user behaviour patterns |
| `RecommendationFeedback` | User feedback on recommendations |

## Stable import paths

Most code should import through the facade:

- `gestura_core::analytics::*`
- `gestura_core::recommendations::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-analytics
cargo clippy -p gestura-core-analytics --all-targets --all-features -- -D warnings
```

