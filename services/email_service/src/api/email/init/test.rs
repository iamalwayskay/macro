use super::*;
use macro_db_migrator::MACRO_DB_MIGRATIONS;
use sqlx::PgPool;

async fn insert_email_link(pool: &PgPool) -> Uuid {
    let link_id = Uuid::now_v7();
    let email_address = format!("sso-grant-{link_id}@example.com");
    sqlx::query!(
        r#"
        INSERT INTO email_links (
            id, macro_id, fusionauth_user_id, email_address, provider
        )
        VALUES ($1, $2, $2, $3, 'GMAIL')
        "#,
        link_id,
        "macro|sso-grant@example.com",
        email_address,
    )
    .execute(pool)
    .await
    .unwrap();
    link_id
}

/// A macro user whose profile email is `email`, as (FusionAuth user id, macro id).
async fn macro_user(pool: &PgPool, email: &str) -> (String, String) {
    let fusion_id = Uuid::now_v7();
    let macro_id = format!("macro|{email}");
    sqlx::query!(
        "INSERT INTO macro_user (id, username, email, stripe_customer_id) VALUES ($1, $2, $3, $4)",
        fusion_id,
        macro_id,
        email,
        format!("cus_{fusion_id}"),
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query!(
        r#"INSERT INTO "User" (id, email, macro_user_id) VALUES ($1, $2, $3)"#,
        macro_id,
        email,
        fusion_id,
    )
    .execute(pool)
    .await
    .unwrap();
    (fusion_id.to_string(), macro_id)
}

async fn gmail_link(
    pool: &PgPool,
    (fusion_id, macro_id): &(String, String),
    mailbox: &str,
) -> Uuid {
    let link_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO email_links (id, macro_id, fusionauth_user_id, email_address, provider)
        VALUES ($1, $2, $3, $4, 'GMAIL')
        "#,
        link_id,
        macro_id,
        fusion_id,
        mailbox,
    )
    .execute(pool)
    .await
    .unwrap();
    link_id
}

/// Resolves the linked email's owner the way `init_user` does, checks the connect
/// takes the data-source path, and returns where it lands.
async fn data_source_target_for(
    pool: &PgPool,
    (fusion_id, macro_id): &(String, String),
    mailbox: &str,
) -> DataSourceTarget {
    let existing_owner =
        match macro_db_client::user::get::get_user_id_by_email(pool.clone(), mailbox).await {
            Ok(owner) => Some(owner),
            Err(sqlx::Error::RowNotFound) => None,
            Err(error) => panic!("owner lookup failed: {error}"),
        };
    assert!(
        existing_owner
            .as_ref()
            .is_none_or(|owner| owner == macro_id),
        "a mailbox owned by another macro user takes the delegation path"
    );
    data_source_target(
        pool,
        &EmailPgRepo::new(pool.clone()),
        fusion_id,
        macro_id,
        existing_owner.as_deref(),
        mailbox,
    )
    .await
    .unwrap()
}

/// A mailbox connected as an extra inbox in one account, which later got its own
/// Macro account. Connecting it from its own account, the OAuth callback finds the
/// Google grant on the first account's FusionAuth user and skips replacing it, so
/// init must not provision an inbox with no grant behind it.
#[sqlx::test(migrator = "MACRO_DB_MIGRATIONS")]
async fn own_mailbox_held_as_an_inbox_in_another_account_is_connected_elsewhere(pool: PgPool) {
    let first_account = macro_user(&pool, "first@example.com").await;
    let existing_link_id = gmail_link(&pool, &first_account, "mailbox@example.com").await;
    let mailbox_account = macro_user(&pool, "mailbox@example.com").await;

    let target = data_source_target_for(&pool, &mailbox_account, "mailbox@example.com").await;

    assert!(
        matches!(target, DataSourceTarget::ConnectedElsewhere { existing_link_id: id } if id == existing_link_id),
        "{target:?}"
    );
}

#[sqlx::test(migrator = "MACRO_DB_MIGRATIONS")]
async fn inbox_under_another_fusionauth_user_of_the_same_account_is_connected_elsewhere(
    pool: PgPool,
) {
    let caller = macro_user(&pool, "owner@example.com").await;
    let other_fusion_user = (Uuid::now_v7().to_string(), caller.1.clone());
    let existing_link_id = gmail_link(&pool, &other_fusion_user, "extra@example.com").await;

    let target = data_source_target_for(&pool, &caller, "extra@example.com").await;

    assert!(
        matches!(target, DataSourceTarget::ConnectedElsewhere { existing_link_id: id } if id == existing_link_id),
        "{target:?}"
    );
}

#[sqlx::test(migrator = "MACRO_DB_MIGRATIONS")]
async fn mailbox_of_no_macro_user_connected_by_another_user_can_be_shared(pool: PgPool) {
    let first = macro_user(&pool, "first@example.com").await;
    let existing_link_id = gmail_link(&pool, &first, "team@example.com").await;
    let second = macro_user(&pool, "second@example.com").await;

    let target = data_source_target_for(&pool, &second, "team@example.com").await;

    assert!(
        matches!(&target, DataSourceTarget::Share(link) if link.id == existing_link_id),
        "{target:?}"
    );
}

#[sqlx::test(migrator = "MACRO_DB_MIGRATIONS")]
async fn callers_own_inbox_is_reconnected(pool: PgPool) {
    let caller = macro_user(&pool, "owner@example.com").await;
    let other = macro_user(&pool, "other@example.com").await;
    gmail_link(&pool, &other, "extra@example.com").await;
    let existing_link_id = gmail_link(&pool, &caller, "extra@example.com").await;

    let target = data_source_target_for(&pool, &caller, "extra@example.com").await;

    assert!(
        matches!(target, DataSourceTarget::Existing { existing_link_id: id } if id == existing_link_id),
        "{target:?}"
    );
}

#[sqlx::test(migrator = "MACRO_DB_MIGRATIONS")]
async fn mailbox_no_account_holds_is_new(pool: PgPool) {
    let caller = macro_user(&pool, "owner@example.com").await;

    for mailbox in ["owner@example.com", "extra@example.com"] {
        let target = data_source_target_for(&pool, &caller, mailbox).await;
        assert!(matches!(target, DataSourceTarget::New), "{target:?}");
    }
}

async fn body_json(response: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[sqlx::test(migrator = "MACRO_DB_MIGRATIONS")]
async fn zero_grant_is_unrecorded(pool: PgPool) {
    let link_id = insert_email_link(&pool).await;
    sqlx::query!(
        "INSERT INTO email_link_google_scopes (link_id) VALUES ($1)",
        link_id,
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(has_unrecorded_google_grant(&pool, link_id).await);
}

#[sqlx::test(migrator = "MACRO_DB_MIGRATIONS")]
async fn missing_side_table_state_is_unrecorded(pool: PgPool) {
    let link_id = insert_email_link(&pool).await;

    assert!(has_unrecorded_google_grant(&pool, link_id).await);
}

#[sqlx::test(migrator = "MACRO_DB_MIGRATIONS")]
async fn recorded_side_table_grant_is_not_unrecorded(pool: PgPool) {
    let link_id = insert_email_link(&pool).await;
    sqlx::query!(
        "INSERT INTO email_link_google_scopes (link_id, grant_version) VALUES ($1, 1)",
        link_id,
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(!has_unrecorded_google_grant(&pool, link_id).await);
}

#[sqlx::test(migrator = "MACRO_DB_MIGRATIONS")]
async fn missing_email_link_is_not_unrecorded(pool: PgPool) {
    assert!(!has_unrecorded_google_grant(&pool, Uuid::now_v7()).await);
}

#[tokio::test]
async fn no_gmail_grant_is_a_coded_400() {
    let response = InitError::NoGmailGrant.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert_eq!(body["code"], NO_GMAIL_GRANT_CODE);
}

#[tokio::test]
async fn mailbox_connected_elsewhere_is_a_coded_400() {
    let response = InitError::MailboxConnectedElsewhere.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert_eq!(body["code"], MAILBOX_CONNECTED_ELSEWHERE_CODE);
}

#[tokio::test]
async fn already_initialized_is_a_coded_400() {
    let response = InitError::AlreadyInitialized.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert_eq!(body["code"], ALREADY_INITIALIZED_CODE);
}

#[test]
fn unusable_provider_grants_classify_as_no_gmail_grant() {
    for error in [EmailApiError::AuthRequired, EmailApiError::Forbidden] {
        assert!(matches!(
            classify_provider_init_error(error),
            InitError::NoGmailGrant
        ));
    }
}

#[test]
fn rate_limiting_classifies_as_http_429() {
    let error = classify_provider_init_error(EmailApiError::RateLimited {
        retry_after: None,
        origin: email_api_client::domain::models::RateLimitOrigin::Provider { concurrency: false },
    });
    assert_eq!(error.status_code(), StatusCode::TOO_MANY_REQUESTS);
}

#[test]
fn a_rate_limited_init_tells_the_client_when_to_retry() {
    let response = InitError::ProviderError(EmailApiError::RateLimited {
        retry_after: Some(std::time::Duration::from_millis(1_200)),
        origin: email_api_client::domain::models::RateLimitOrigin::Local,
    })
    .into_response();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("2")
    );
}

#[tokio::test]
async fn provider_diagnostics_are_not_returned_to_clients() {
    let response = InitError::ProviderError(EmailApiError::Permanent {
        message: "sanitized provider body".to_string(),
    })
    .into_response();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(bytes, "Email provider operation failed");
}

#[test]
fn only_shared_inbox_conflict_keeps_the_in_progress_link() {
    // SharedInboxConflict is held open for the force_share retry, so its row must be kept;
    // every other terminal failure must clean up the row so it stops counting toward the
    // /link/gmail start cap. The delete itself is covered by macro_db_client's own tests.
    assert!(
        !should_clean_up_in_progress_link(&InitError::SharedInboxConflict {
            email_address: "shared@example.com".to_string(),
            existing_owner_email: "owner@example.com".to_string(),
            existing_link_id: Uuid::new_v4(),
        }),
        "SharedInboxConflict must keep the row for the force_share retry"
    );

    for err in [
        InitError::AlreadyInitialized,
        InitError::NoGmailGrant,
        InitError::MailboxConnectedElsewhere,
        InitError::EnqueueError,
        InitError::BadRequest("bad".to_string()),
        InitError::ProviderError(EmailApiError::AuthRequired),
    ] {
        assert!(
            should_clean_up_in_progress_link(&err),
            "terminal failure {err:?} should clean up the in_progress row"
        );
    }
}

/// The grant alone cannot say whether the user wanted calendar: consent
/// requests carry `include_granted_scopes=true`, so Google re-issues the
/// calendar scopes of an earlier grant on a plain Gmail reconnect. Only what
/// the request asked for decides.
#[test]
fn calendar_intent_follows_the_consent_request_not_the_grant() {
    let calendar_scopes = calendar_events::domain::models::GOOGLE_CALENDAR_SCOPES
        .map(str::to_owned)
        .to_vec();
    let gmail_scope = "https://www.googleapis.com/auth/gmail.modify".to_owned();
    let in_progress = |requested: Vec<String>| InProgressUserLink {
        email_provider: "GMAIL".into(),
        macro_user_id: Uuid::now_v7(),
        linked_email: None,
        requested_google_scopes: requested,
        granted_google_scopes: [vec![gmail_scope.clone()], calendar_scopes.clone()].concat(),
    };

    assert!(matches!(
        CompletedGoogleGrant::from_in_progress(&in_progress(vec![gmail_scope.clone()])).intent,
        CalendarGrantIntent::Incidental
    ));
    assert!(matches!(
        CompletedGoogleGrant::from_in_progress(&in_progress(
            [vec![gmail_scope.clone()], calendar_scopes.clone()].concat()
        ))
        .intent,
        CalendarGrantIntent::CalendarRequested
    ));
}
