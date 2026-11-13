use super::*;

#[test]
fn parses_booleans_and_rejects_other_values_and_keys() {
    let values = parse(r#"{"enable-crm": true, "disable-x": false}"#).unwrap();
    assert_eq!(values.get("enable-crm"), Some(&true));
    assert_eq!(values.get("disable-x"), Some(&false));
    assert!(parse(r#"{"enable-crm": "on"}"#).is_err());
    assert!(parse(r#"{"bad`key": true}"#).is_err());
    assert!(parse("[]").is_err());
}

#[test]
fn caddy_site_serves_a_posthog_flags_response() {
    let site = caddy_site(&BTreeMap::from([("enable-crm".to_string(), true)]));
    let body = site
        .split('`')
        .nth(1)
        .expect("the response body is a backtick literal");
    let response: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        response,
        serde_json::json!({
            "flags": {"enable-crm": {"key": "enable-crm", "enabled": true}},
            "errorsWhileComputingFlags": false,
        })
    );
    assert!(site.contains("http://:8099"));
}

#[test]
fn stands_in_only_for_the_placeholder_key_on_a_local_stack() {
    let placeholder = BTreeMap::from([(
        "POSTHOG_API_KEY".to_string(),
        POSTHOG_PLACEHOLDER_KEY.to_string(),
    )]);
    let real = BTreeMap::from([("POSTHOG_API_KEY".to_string(), "phc_real".to_string())]);
    assert!(stands_in(Mode::Local, &placeholder));
    assert!(!stands_in(Mode::Local, &real));
    assert!(!stands_in(Mode::Dev, &placeholder));
}
