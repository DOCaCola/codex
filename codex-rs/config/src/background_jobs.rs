use std::num::NonZeroU64;
use std::time::Duration;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct BackgroundJobsConfigToml {
    /// Auto/background deadline on unattended hosts, from process start. Default: 1800000.
    pub unattended_deadline_ms: Option<NonZeroU64>,
    /// Managed foreground deadline. Default: 300000.
    pub foreground_timeout_ms: Option<NonZeroU64>,
    /// Initial auto wait, without the legacy Windows floor. Default: 10000.
    pub auto_yield_ms: Option<NonZeroU64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackgroundJobsConfig {
    pub unattended_deadline: Duration,
    pub foreground_timeout: Duration,
    pub auto_yield: Duration,
}

impl Default for BackgroundJobsConfig {
    fn default() -> Self {
        Self {
            unattended_deadline: Duration::from_millis(1_800_000),
            foreground_timeout: Duration::from_millis(300_000),
            auto_yield: Duration::from_millis(10_000),
        }
    }
}

impl BackgroundJobsConfigToml {
    pub fn resolve(&self) -> std::io::Result<BackgroundJobsConfig> {
        let defaults = BackgroundJobsConfig::default();
        Ok(BackgroundJobsConfig {
            unattended_deadline: milliseconds("unattended_deadline_ms", self.unattended_deadline_ms, defaults.unattended_deadline.as_millis() as u64)?,
            foreground_timeout: milliseconds("foreground_timeout_ms", self.foreground_timeout_ms, defaults.foreground_timeout.as_millis() as u64)?,
            auto_yield: milliseconds("auto_yield_ms", self.auto_yield_ms, defaults.auto_yield.as_millis() as u64)?,
        })
    }
}

fn milliseconds(key: &str, value: Option<NonZeroU64>, default: u64) -> std::io::Result<Duration> {
    let duration = Duration::from_millis(value.map(NonZeroU64::get).unwrap_or(default));
    if std::time::Instant::now().checked_add(duration).is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("background_jobs.{key} is too large"),
        ));
    }
    Ok(duration)
}

#[cfg(test)]
#[path = "background_jobs_tests.rs"]
mod tests;
