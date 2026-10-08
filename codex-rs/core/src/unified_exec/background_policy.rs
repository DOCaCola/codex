//! Session-captured host defaults; model deadlines override host policy.
use std::time::Duration;

use codex_config::BackgroundJobsConfig;
use codex_protocol::protocol::SessionSource;
use serde::Deserialize;
use serde::Deserializer;

use super::managed::ExecutionMode;

pub(crate) use crate::background_jobs::BackgroundJobHostClass as HostClass;

impl HostClass {
    pub(crate) fn for_source(source: &SessionSource, owner: Self) -> Self {
        match source {
            SessionSource::Exec => Self::Unattended,
            SessionSource::Cli
            | SessionSource::VSCode
            | SessionSource::Mcp
            | SessionSource::Custom(_)
            | SessionSource::Unknown => Self::Interactive,
            SessionSource::SubAgent(_) | SessionSource::Internal(_) => owner,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct BackgroundJobPolicy {
    pub enabled: bool,
    pub host: HostClass,
    pub defaults: BackgroundJobsConfig,
}

impl BackgroundJobPolicy {
    pub(crate) fn capture(config: &crate::config::Config, source: &SessionSource) -> Self {
        Self {
            enabled: config.features.enabled(codex_features::Feature::BackgroundJobs),
            host: HostClass::for_source(
                source, config.background_jobs_host.unwrap_or(HostClass::Interactive),
            ),
            defaults: config.background_jobs,
        }
    }

    pub(crate) fn deadline_guidance(&self) -> String {
        let background = match self.host {
            HostClass::Interactive => "no implicit deadline".to_owned(),
            HostClass::Unattended => format!("a default deadline of {} ms", self.defaults.unattended_deadline.as_millis()),
        };
        format!("Foreground defaults to {} ms; auto/background have {background}. Deadlines count from process start; explicit timeout_ms overrides every default. Dependencies keep their requested mode's deadline. Auto initially waits {} ms unless yield_time_ms is supplied.",
            self.defaults.foreground_timeout.as_millis(), self.defaults.auto_yield.as_millis())
    }

    pub(crate) fn deadline(&self, mode: ExecutionMode, explicit: Option<Duration>) -> Option<Duration> {
        explicit.or(match mode {
            ExecutionMode::Foreground => Some(self.defaults.foreground_timeout),
            ExecutionMode::Auto | ExecutionMode::Background => match self.host {
                HostClass::Interactive => None,
                HostClass::Unattended => Some(self.defaults.unattended_deadline),
            },
            ExecutionMode::Interactive => None,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) enum YieldSource { Legacy, SessionDefault }

impl YieldSource {
    pub(crate) fn initial_wait_ms(self, millis: u64) -> u64 {
        match self {
            Self::Legacy => super::clamp_yield_time(millis),
            Self::SessionDefault => millis,
        }
    }
}

/// Omitted auto yields use captured policy; explicit numbers keep legacy bounds.
/// Deserialize as u64 so explicit null/negative values retain the legacy error.
#[derive(Debug, Default)]
pub(crate) struct InitialYield(pub Option<u64>);

impl InitialYield {
    pub(crate) fn resolve(self, policy: &BackgroundJobPolicy, mode: Option<ExecutionMode>) -> (u64, YieldSource) {
        if self.0.is_none() && policy.enabled && mode == Some(ExecutionMode::Auto) {
            (policy.defaults.auto_yield.as_millis() as u64, YieldSource::SessionDefault)
        } else {
            (self.0.unwrap_or(10_000), YieldSource::Legacy)
        }
    }
}

impl<'de> Deserialize<'de> for InitialYield {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u64::deserialize(deserializer).map(|value| Self(Some(value)))
    }
}

#[cfg(test)]
#[path = "background_policy_tests.rs"]
mod tests;
