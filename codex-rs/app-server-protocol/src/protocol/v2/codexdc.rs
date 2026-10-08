use crate::JsonSchema;
use crate::TS;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CodexdcCapabilitiesParams {}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CodexdcCapabilitiesResponse {
    /// Fork identifier used by CodexDC to recognize the attached backend.
    pub fork: String,
    /// Version of the app-server build.
    pub version: String,
    /// Supported fork features and their integer protocol revisions.
    pub protocol: BTreeMap<String, u32>,
    /// Effective feature enablement in the current base configuration.
    pub features: CodexdcFeatures,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CodexdcFeatures {
    pub background_jobs: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CodexdcBackgroundJobsListParams {
    pub thread_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CodexdcBackgroundWorkState {
    pub background_jobs_enabled: bool,
    pub job_timers: Vec<CodexdcBackgroundJobTimer>,
    pub idle_continuation: bool,
    pub unattended_deadline_ms: u64,
    pub thread_id: String,
    pub pending: bool,
    pub active: bool,
    pub last_turn_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CodexdcBackgroundJobTimer {
    pub item_id: String,
    pub process_id: String,
    #[ts(type = "number")]
    pub started_at_ms: i64,
    pub explicit_timeout_seconds: Option<u64>,
    pub duration_seconds: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CodexdcBackgroundJobsListResponse {
    pub enabled: bool,
    pub data: Vec<BackgroundJob>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct BackgroundJob {
    pub job_id: String,
    pub item_id: String,
    /// Reusable terminal handle; display only, not a job identity.
    pub process_id: String,
    pub mode: BackgroundJobMode,
    pub state: BackgroundJobState,
    pub started_at_ms: i64,
    pub timeout_ms: Option<i64>,
    pub deadline_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub exit_code: Option<i32>,
    pub delivery: BackgroundJobDelivery,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum BackgroundJobMode {
    Auto,
    Background,
    Foreground,
    Interactive,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum BackgroundJobState {
    Running,
    Completed,
    Failed,
    TimedOut,
    Stopped,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum BackgroundJobDelivery {
    Pending,
    Delivered,
    None,
}
