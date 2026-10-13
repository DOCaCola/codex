//! Export gating for a payload shared by an experimental response and notification.

use super::*;
use crate::TS;
use pretty_assertions::assert_eq;

#[test]
fn excluded_notification_payload_follows_its_method_export_gate() -> Result<()> {
    let methods = methods_with_excluded_payloads(&super::super::experimental_method_types())?;
    assert_eq!(methods, vec!["codexdc/backgroundWork/changed".to_owned()]);
    let methods: Vec<&str> = methods.iter().map(String::as_str).collect();
    for generated in [
        crate::ServerNotification::export_to_string()?,
        crate::ServerNotificationEnvelope::export_to_string()?,
    ] {
        assert!(generated.contains("codexdc/backgroundWork/changed"));
        let filtered = super::super::filter_request_ts_contents(generated, &methods);
        assert!(!filtered.contains("CodexdcBackgroundWorkState"));
        assert!(!filtered.contains("codexdc/backgroundWork/changed"));
        for method in [
            "thread/started",
            "thread/prediction/updated",
            "process/exited",
        ] {
            assert!(filtered.contains(method));
        }
    }
    let mut schema = serde_json::to_value(schemars::schema_for!(crate::ServerNotification))?;
    super::super::filter_experimental_schema(&mut schema)?;
    assert!(!schema.to_string().contains("CodexdcBackgroundWorkState"));
    Ok(())
}

#[test]
fn stable_tree_and_filesystem_exports_agree_for_notification_envelopes() -> Result<()> {
    let tree = crate::schema_fixtures::generate_typescript_schema_fixture_subtree_for_tests()?;
    let out = tempfile::tempdir()?;
    super::super::generate_ts_with_options(
        &out.path().join("typescript"),
        /*prettier*/ None,
        super::super::GenerateTsOptions::default(),
    )?;
    let filesystem = crate::schema_fixtures::read_schema_fixture_subtree(out.path(), "typescript")?;
    for name in ["ServerNotification.ts", "ServerNotificationEnvelope.ts"] {
        let name = std::path::Path::new(name);
        let generated = String::from_utf8(filesystem[name].clone())?;
        assert_eq!(String::from_utf8(tree[name].clone())?, generated);
        assert!(!generated.contains("CodexdcBackgroundWorkState"));
    }
    Ok(())
}
