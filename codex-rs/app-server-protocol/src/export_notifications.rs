//! Keep notification unions consistent with the payload types excluded from stable exports.
//!
//! Upstream notification `ExperimentalApi` annotations also govern runtime delivery and do not
//! by themselves remove existing notification contracts from stable schemas. A notification
//! sharing an excluded experimental method payload must follow that payload's export gate.

use anyhow::Context;
use anyhow::Result;
use std::collections::HashSet;

pub(super) fn methods_with_excluded_payloads(
    excluded_types: &HashSet<String>,
) -> Result<Vec<String>> {
    let schema = serde_json::to_value(schemars::schema_for!(crate::ServerNotification))?;
    let variants = schema["oneOf"]
        .as_array()
        .context("ServerNotification schema must contain oneOf")?;
    let mut methods = Vec::new();
    for variant in variants {
        let Some(reference) = variant["properties"]["params"]["$ref"].as_str() else {
            continue;
        };
        let payload = reference
            .strip_prefix("#/definitions/")
            .context("notification payload must reference a local definition")?;
        if excluded_types.contains(payload) {
            methods.push(
                variant["properties"]["method"]["enum"][0]
                    .as_str()
                    .context("notification method discriminator")?
                    .to_owned(),
            );
        }
    }
    Ok(methods)
}

pub(super) fn filter_method_union(body: &str, excluded_methods: &HashSet<&str>) -> String {
    super::split_top_level(body, '&')
        .into_iter()
        .map(|part| {
            if let Some(union) = part
                .strip_prefix('(')
                .and_then(|part| part.strip_suffix(')'))
            {
                return format!("({})", filter_method_union(union, excluded_methods));
            }
            super::split_top_level(&part, '|')
                .into_iter()
                .filter(|arm| {
                    super::extract_discriminator_from_arm(arm, "method")
                        .is_none_or(|method| !excluded_methods.contains(method.as_str()))
                })
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect::<Vec<_>>()
        .join(" & ")
}

#[cfg(test)]
#[path = "export_notifications_tests.rs"]
mod tests;
