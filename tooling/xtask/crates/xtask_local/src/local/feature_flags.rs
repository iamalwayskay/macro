//! Feature flags for a local stack without a PostHog project.
//!
//! A `--no-doppler` stack gives the services a placeholder PostHog key, so
//! every flag load would otherwise go to PostHog's cloud with a key it rejects.
//! Such a stack answers flag loads itself instead: the proxy serves PostHog's
//! `/flags` response on a port only the services network reaches, and the
//! services' `POSTHOG_HOST` points there.
//!
//! The values come from the JSON file named by `MACRO_LOCAL_FEATURE_FLAGS`
//! (PostHog flag keys to `true` or `false`); without one every flag reads
//! off. The same values reach the web bundle as `VITE_LOCAL_FEATURE_FLAGS`.
//! A stack with a real PostHog key (Doppler, `--env-file`) keeps PostHog.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use super::Mode;
use super::local_env::POSTHOG_PLACEHOLDER_KEY;

/// Names the JSON file of local flag values.
pub const FILE_ENV: &str = "MACRO_LOCAL_FEATURE_FLAGS";
/// The proxy port that answers flag loads, unpublished.
const PORT: u16 = 8099;

/// The configured flag values, or none when `MACRO_LOCAL_FEATURE_FLAGS` is unset.
pub fn values() -> Result<BTreeMap<String, bool>> {
    let Some(path) = std::env::var_os(FILE_ENV).filter(|path| !path.is_empty()) else {
        return Ok(BTreeMap::new());
    };
    let path = PathBuf::from(path);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {FILE_ENV} file {}", path.display()))?;
    parse(&text).with_context(|| format!("parsing {FILE_ENV} file {}", path.display()))
}

fn parse(text: &str) -> Result<BTreeMap<String, bool>> {
    let values: BTreeMap<String, serde_json::Value> = serde_json::from_str(text)?;
    values
        .into_iter()
        .map(|(key, value)| match value {
            serde_json::Value::Bool(enabled) if is_flag_key(&key) => Ok((key, enabled)),
            serde_json::Value::Bool(_) => bail!("`{key}` is not a flag key"),
            _ => bail!("flag `{key}` must be true or false"),
        })
        .collect()
}

/// PostHog keys as Macro names them: letters, digits, `-` and `_`.
fn is_flag_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Whether this stack answers flag loads itself: a local stack whose services
/// would otherwise send the placeholder key to PostHog.
pub fn stands_in(mode: Mode, env: &BTreeMap<String, String>) -> bool {
    mode == Mode::Local
        && env.get("POSTHOG_API_KEY").map(String::as_str) == Some(POSTHOG_PLACEHOLDER_KEY)
}

/// What the services get when the stack stands in for PostHog.
pub fn env() -> [(String, String); 1] {
    [("POSTHOG_HOST".into(), format!("http://proxy:{PORT}"))]
}

/// The frontend's flag values, for a bundle built without a PostHog key.
pub fn frontend_env() -> Result<Option<(&'static str, String)>> {
    if std::env::var_os(FILE_ENV).is_none_or(|path| path.is_empty()) {
        return Ok(None);
    }
    Ok(Some((
        "VITE_LOCAL_FEATURE_FLAGS",
        serde_json::to_string(&values()?)?,
    )))
}

/// The proxy site that answers PostHog's `/flags` request with the configured
/// values, the same for every user. The body is a Caddyfile backtick literal;
/// flag keys and JSON booleans never contain a backtick.
pub fn caddy_site(values: &BTreeMap<String, bool>) -> String {
    let flags: serde_json::Map<String, serde_json::Value> = values
        .iter()
        .map(|(key, enabled)| {
            (
                key.clone(),
                serde_json::json!({ "key": key, "enabled": enabled }),
            )
        })
        .collect();
    let body = serde_json::json!({ "flags": flags, "errorsWhileComputingFlags": false });
    format!(
        "\nhttp://:{PORT} {{\n    handle /flags/* {{\n        header Content-Type application/json\n        respond `{body}` 200\n    }}\n    respond 404\n}}\n"
    )
}

#[cfg(test)]
mod test;
