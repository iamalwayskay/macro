//! `POST /user_notifications/send`: send a notification the caller wrote.

#[cfg(test)]
mod test;

use std::sync::Arc;

use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::http::header::RETRY_AFTER;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use bots::outbound::pg_bots_repo::PgBotsRepo;
use chrono::Utc;
use entity_access::domain::service::EntityAccessServiceImpl;
use entity_access::outbound::PgAccessRepository;
use macro_authorization::{MacroAuthorizationExtractor, MacroAuthorizationState, UserOrInternal};
use macro_user_id::user_id::MacroUserIdStr;
use messages::domain::ports::NoMessageEventPublisher;
use messages::domain::service::MessageService;
use messages::outbound::pg_message_repo::PgMessageRepository;
use model_entity::EntityType;
use model_error_response::ErrorResponse;
use notification::domain::service::SqsNotificationIngress;
use notification::outbound::queue::SqsQueue;
use notification::outbound::rate_limit::RedisRateLimitAdapter;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::context::AuthorizationService;
use crate::manual::allowance::HourlyAllowance;
use crate::manual::bots::BotAgentProfiles;
use crate::manual::channel_messages::ChannelMessageLinks;
use crate::manual::entity_access::EntityAccessLinks;
use crate::manual::flags::NotificationFlags;
use crate::manual::sessions::PgSessionListing;
use crate::manual::{
    ManualNotificationDraft, ManualNotificationError, ManualNotificationOutcome,
    ManualNotifications, ManualSender,
};

pub(crate) type EntityAccess = EntityAccessServiceImpl<PgAccessRepository>;

/// Reads messages only: nothing is posted through it, so nothing is published.
pub(crate) type MessageReads = MessageService<PgMessageRepository, NoMessageEventPublisher>;

pub(crate) type Notifications = ManualNotifications<
    NotificationFlags,
    EntityAccessLinks<EntityAccess>,
    PgSessionListing,
    ChannelMessageLinks<EntityAccess, MessageReads>,
    BotAgentProfiles<PgBotsRepo>,
    HourlyAllowance<RedisRateLimitAdapter<redis::Client>>,
    SqsNotificationIngress<SqsQueue>,
>;

/// State for the manual notification route.
pub(crate) struct ManualNotificationState {
    pub notifications: Arc<Notifications>,
    pub authorization_state: MacroAuthorizationState<AuthorizationService>,
}

impl Clone for ManualNotificationState {
    fn clone(&self) -> Self {
        Self {
            notifications: Arc::clone(&self.notifications),
            authorization_state: self.authorization_state.clone(),
        }
    }
}

impl FromRef<ManualNotificationState> for MacroAuthorizationState<AuthorizationService> {
    fn from_ref(state: &ManualNotificationState) -> Self {
        state.authorization_state.clone()
    }
}

/// What a manual notification opens.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualNotificationLink {
    /// One of document, chat, channel, channel_message, project, initiative,
    /// email_thread, calendar_event, crm_company, crm_contact or
    /// agent_session: the kinds the inbox lists. A channel message is filed
    /// under its channel and opens at the message. An agent session must be
    /// listed; an email thread shows only while it is Signal-important and
    /// not shared.
    pub entity_type: EntityType,
    /// The entity's id.
    pub entity_id: String,
}

/// A notification written by the caller.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendManualNotificationRequest {
    /// Who gets it, at most 100; the caller when omitted or empty. Anyone
    /// else who cannot already see `link` is skipped, never given access.
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub recipient_ids: Vec<MacroUserIdStr<'static>>,
    /// The headline, at most 120 characters.
    pub title: String,
    /// More detail, at most 1000 characters.
    #[serde(default)]
    pub body: Option<String>,
    /// What the notification opens; the caller must be able to see it.
    pub link: ManualNotificationLink,
}

pub(crate) fn router<S>(state: ManualNotificationState) -> Router<S> {
    Router::new()
        .route("/send", post(send_manual_notification))
        .with_state(state)
}

/// Send a notification the caller wrote about an entity, to themselves or to
/// people who can already see that entity. Recipients who cannot see it are
/// skipped and returned. Delivered like any other notification and subject to
/// each recipient's notification preferences. The same notification sent again
/// in the same ten-minute window of the clock is delivered once; a repeat that
/// falls in the next window is delivered again. A sender can notify at most
/// 1000 recipients an hour. Refused with 403 while the
/// `enable-agent-notifications` flag is off for the caller.
#[utoipa::path(
    post,
    operation_id = "send_manual_notification",
    path = "/v1/user_notifications/send",
    request_body = SendManualNotificationRequest,
    responses(
        (status = 200, description = "Queued for the recipients who can see the link", body = ManualNotificationOutcome),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, description = "Agent notifications are not enabled for the caller", body = ErrorResponse),
        (status = 404, description = "The link does not exist or the caller cannot see it", body = ErrorResponse),
        (status = 429, description = "The sender has notified too many recipients this hour; `Retry-After` says when to try again", body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 503, description = "Whether agent notifications are enabled for the caller could not be checked; try again", body = ErrorResponse),
    )
)]
pub(crate) async fn send_manual_notification(
    State(state): State<ManualNotificationState>,
    authorization: MacroAuthorizationExtractor<AuthorizationService, UserOrInternal>,
    Json(request): Json<SendManualNotificationRequest>,
) -> Response {
    let user = authorization.authorization.user;

    let sent = state
        .notifications
        .send(
            ManualSender {
                user: user.macro_user_id,
                agent_id: user.agent_id,
            },
            ManualNotificationDraft {
                recipients: request.recipient_ids,
                title: request.title,
                body: request.body,
                link: request
                    .link
                    .entity_type
                    .with_entity_string(request.link.entity_id),
            },
            Utc::now(),
        )
        .await;

    match sent {
        Ok(outcome) => Json(outcome).into_response(),
        Err(error) => error_response(error),
    }
}

fn error_response(error: ManualNotificationError) -> Response {
    let message = |status: StatusCode, message: String| {
        (
            status,
            Json(ErrorResponse {
                message: message.into(),
            }),
        )
    };
    match &error {
        ManualNotificationError::NotEnabled => {
            message(StatusCode::FORBIDDEN, error.to_string()).into_response()
        }
        ManualNotificationError::FlagsUnavailable(report) => {
            tracing::warn!(error = ?report, "could not read the agent notifications flag");
            message(
                StatusCode::SERVICE_UNAVAILABLE,
                "could not check whether agent notifications are enabled; try again".to_string(),
            )
            .into_response()
        }
        ManualNotificationError::EmptyTitle
        | ManualNotificationError::TitleTooLong
        | ManualNotificationError::BodyTooLong
        | ManualNotificationError::TooManyRecipients
        | ManualNotificationError::UnlinkableEntity(_)
        | ManualNotificationError::NotShownInInbox(_)
        | ManualNotificationError::UnlistedAgentSession => {
            message(StatusCode::BAD_REQUEST, error.to_string()).into_response()
        }
        ManualNotificationError::LinkNotFound => {
            message(StatusCode::NOT_FOUND, error.to_string()).into_response()
        }
        ManualNotificationError::AllowanceExceeded(exceeded) => (
            [(RETRY_AFTER, exceeded.retry_after.as_secs().to_string())],
            message(StatusCode::TOO_MANY_REQUESTS, error.to_string()),
        )
            .into_response(),
        ManualNotificationError::Internal(report) => {
            tracing::error!(error = ?report, "could not send a manual notification");
            message(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not send the notification".to_string(),
            )
            .into_response()
        }
    }
}
