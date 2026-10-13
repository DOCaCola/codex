//! Typed table defaults, overrides and positive-value errors.
use crate::config_toml::ConfigToml;
use pretty_assertions::assert_eq;

#[test]
fn background_jobs_table_defaults_and_overrides_resolve() {
    for (text, expected) in [
        ("", [1_800_000, 300_000, 10_000]),
        ("[background_jobs]", [1_800_000, 300_000, 10_000]),
        (
            "[background_jobs]\nunattended_deadline_ms=7200000\nforeground_timeout_ms=900000\nauto_yield_ms=25",
            [7_200_000, 900_000, 25],
        ),
    ] {
        let config: ConfigToml = toml::from_str(text).unwrap();
        let resolved = config
            .background_jobs
            .unwrap_or_default()
            .resolve()
            .unwrap();
        assert_eq!(
            [
                resolved.unattended_deadline.as_millis(),
                resolved.foreground_timeout.as_millis(),
                resolved.auto_yield.as_millis()
            ],
            expected
        );
    }
}

#[test]
fn background_jobs_table_rejects_zero_and_negative_values() {
    for (key, value) in [
        "unattended_deadline_ms",
        "foreground_timeout_ms",
        "auto_yield_ms",
    ]
    .into_iter()
    .flat_map(|key| [0, -1].map(|value| (key, value)))
    {
        let error =
            toml::from_str::<ConfigToml>(&format!("[background_jobs]\n{key}={value}")).unwrap_err();
        assert!(error.to_string().contains(key));
        assert!(error.to_string().contains("nonzero") || error.to_string().contains("u64"));
    }
}
