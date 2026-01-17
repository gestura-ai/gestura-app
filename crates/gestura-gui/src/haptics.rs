//! Haptics interface scaffolding and BLE hooks (Stage 2+)
//! Provides a dual-auth gated interface to trigger haptic feedback via the ring.

use crate::AppError;

/// High-level patterns the app can trigger.
#[derive(Debug, Clone, Copy)]
pub enum HapticPattern {
    Click,
    Pulse,
    Ramp,
    Heartbeat,
    Notification,
    Alert,
    Custom(u8),
}

/// Request describing a haptic action.
#[derive(Debug, Clone)]
pub struct HapticRequest {
    pub pattern: HapticPattern,
    /// Intensity from 0.0 to 1.0
    pub intensity: f32,
    /// Duration in milliseconds
    pub duration_ms: u32,
    /// Repeat count (0 = single, >0 = repeat n times)
    pub repeat_count: u8,
    /// Delay between repeats in milliseconds
    pub repeat_delay_ms: u32,
}

/// Advanced haptic pattern builder
pub struct HapticPatternBuilder {
    pattern: HapticPattern,
    intensity: f32,
    duration_ms: u32,
    repeat_count: u8,
    repeat_delay_ms: u32,
}

impl HapticPatternBuilder {
    pub fn new(pattern: HapticPattern) -> Self {
        Self {
            pattern,
            intensity: 0.5,
            duration_ms: 100,
            repeat_count: 0,
            repeat_delay_ms: 100,
        }
    }

    pub fn intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity.clamp(0.0, 1.0);
        self
    }

    pub fn duration(mut self, duration_ms: u32) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    pub fn repeat(mut self, count: u8, delay_ms: u32) -> Self {
        self.repeat_count = count;
        self.repeat_delay_ms = delay_ms;
        self
    }

    pub fn build(self) -> HapticRequest {
        HapticRequest {
            pattern: self.pattern,
            intensity: self.intensity,
            duration_ms: self.duration_ms,
            repeat_count: self.repeat_count,
            repeat_delay_ms: self.repeat_delay_ms,
        }
    }
}

/// Predefined haptic patterns
impl HapticRequest {
    pub fn click() -> Self {
        HapticPatternBuilder::new(HapticPattern::Click)
            .intensity(0.7)
            .duration(50)
            .build()
    }

    pub fn notification() -> Self {
        HapticPatternBuilder::new(HapticPattern::Notification)
            .intensity(0.6)
            .duration(200)
            .repeat(2, 300)
            .build()
    }

    pub fn alert() -> Self {
        HapticPatternBuilder::new(HapticPattern::Alert)
            .intensity(1.0)
            .duration(500)
            .repeat(3, 200)
            .build()
    }

    pub fn heartbeat() -> Self {
        HapticPatternBuilder::new(HapticPattern::Heartbeat)
            .intensity(0.4)
            .duration(100)
            .repeat(10, 600)
            .build()
    }
}

/// Dual-auth token indicating user granted app-level permission for haptics.
#[derive(Debug, Clone)]
pub struct HapticAuthToken(pub String);

/// Trait abstracting the haptics transport (BLE ring in production)
#[async_trait::async_trait]
pub trait HapticInterface: Send + Sync {
    /// Send a haptic request, gated by app-level auth and optionally MCP auth.
    async fn send(&self, auth: &HapticAuthToken, req: &HapticRequest) -> Result<(), AppError>;
}

/// Mock implementation for tests and early UI wiring.
pub struct MockHaptics;

#[async_trait::async_trait]
impl HapticInterface for MockHaptics {
    async fn send(&self, _auth: &HapticAuthToken, _req: &HapticRequest) -> Result<(), AppError> {
        Ok(())
    }
}
