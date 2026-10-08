//! Written, not run: every source, inheritance, precedence and captured defaults.
use super::*;
use codex_protocol::protocol::InternalSessionSource;
use codex_protocol::protocol::SubAgentSource;
use pretty_assertions::assert_eq;

#[test]
fn background_host_classifies_every_source_and_inherits_root_or_owner() {
    for owner in [HostClass::Interactive, HostClass::Unattended] {
        for source in [SessionSource::Cli, SessionSource::VSCode, SessionSource::Mcp,
            SessionSource::Custom("atlas".into()), SessionSource::Unknown] {
            assert_eq!(HostClass::for_source(&source, owner), HostClass::Interactive);
        }
        assert_eq!(HostClass::for_source(&SessionSource::Exec, owner), HostClass::Unattended);
        for source in [SubAgentSource::Review, SubAgentSource::Compact,
            SubAgentSource::MemoryConsolidation, SubAgentSource::Other("worker".into()),
            SubAgentSource::ThreadSpawn { parent_thread_id: Default::default(), depth: 2,
                agent_path: None, agent_nickname: None, agent_role: None }] {
            assert_eq!(HostClass::for_source(&SessionSource::SubAgent(source), owner), owner);
        }
        for source in [InternalSessionSource::Guardian, InternalSessionSource::MemoryConsolidation] {
            assert_eq!(HostClass::for_source(&SessionSource::Internal(source), owner), owner);
        }
    }
}

#[tokio::test]
async fn background_policy_captures_defaults_timeout_precedence_and_yield_origin() {
    let (_, turn) = crate::session::tests::make_session_and_context().await;
    let mut config = turn.config.as_ref().clone();
    config.features.enable(codex_features::Feature::BackgroundJobs).unwrap();
    let unattended = BackgroundJobPolicy::capture(&config, &SessionSource::Exec);
    let interactive = BackgroundJobPolicy::capture(&config, &SessionSource::Mcp);
    config.background_jobs.unattended_deadline = Duration::from_millis(1);
    config.features.disable(codex_features::Feature::BackgroundJobs).unwrap();
    assert!(unattended.enabled);
    for mode in [ExecutionMode::Auto, ExecutionMode::Background] {
        assert_eq!(unattended.deadline(mode, /*explicit*/ None), Some(Duration::from_secs(1800)));
        assert_eq!(interactive.deadline(mode, /*explicit*/ None), None);
    }
    for mode in [ExecutionMode::Auto, ExecutionMode::Background, ExecutionMode::Foreground] {
        assert_eq!(unattended.deadline(mode, Some(Duration::from_secs(7))), Some(Duration::from_secs(7)));
        assert_eq!(interactive.deadline(mode, Some(Duration::from_secs(7))), Some(Duration::from_secs(7)));
    }
    assert_eq!(unattended.deadline(ExecutionMode::Foreground, /*explicit*/ None), Some(Duration::from_secs(300)));
    let mut policy = unattended;
    policy.defaults.auto_yield = Duration::from_millis(25);
    for enabled in [false, true] {
        policy.enabled = enabled;
        for explicit in [None, Some(1), Some(600_000)] {
            let (millis, source) = InitialYield(explicit).resolve(&policy, Some(ExecutionMode::Auto));
            let expected = if enabled && explicit.is_none() { 25 }
                else { super::super::clamp_yield_time(explicit.unwrap_or(10_000)) };
            assert_eq!(source.initial_wait_ms(millis), expected);
        }
        let (millis, source) = InitialYield(None).resolve(&policy, Some(ExecutionMode::Interactive));
        assert_eq!(source.initial_wait_ms(millis), super::super::clamp_yield_time(10_000));
    }
}

#[test]
fn requested_auto_yield_distinguishes_omission_and_explicit_value_without_null_fallback() {
    #[derive(Deserialize)]
    struct Args { #[serde(default)] yield_time_ms: InitialYield }
    let omitted: Args = serde_json::from_str("{}").unwrap();
    let explicit: Args = serde_json::from_str("{\"yield_time_ms\":25}").unwrap();
    assert_eq!((omitted.yield_time_ms.0, explicit.yield_time_ms.0), (None, Some(25)));
    assert!(serde_json::from_str::<Args>("{\"yield_time_ms\":null}").is_err());
    assert!(serde_json::from_str::<Args>("{\"yield_time_ms\":-1}").is_err());
}
