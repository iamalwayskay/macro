//! Manual notifications: a person, or agent code acting for them, writes a
//! notification about an entity and sends it to themselves or to people who
//! can already see that entity. Delivery goes through the notification ingress
//! like every other kind.
//!
//! Two guards keep a looping program from flooding people: an identical
//! notification repeated within [`DEDUPE_WINDOW`] is delivered once, and each
//! sender has an hourly allowance counted per recipient.
//!
//! Sending is behind the `enable-agent-notifications` flag, read for the
//! sender ([`flags`]).

pub(crate) mod allowance;
pub(crate) mod bots;
pub(crate) mod channel_messages;
pub(crate) mod entity_access;
pub(crate) mod flags;
pub(crate) mod sessions;
#[cfg(test)]
mod test;

use std::collections::HashSet;
use std::future::Future;
use std::time::Duration;

use bot_id::BotId;
use chrono::{DateTime, TimeDelta, Utc};
use feature_flags::FeatureFlags;
use macro_user_id::user_id::MacroUserIdStr;
use model_entity::{Entity, EntityType};
use model_notifications::{ManualMetadata, ManualNotificationAgent, ManualNotificationMessage};
use notification::domain::models::SendNotificationRequestBuilder;
use notification::domain::service::NotificationIngress;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use self::flags::AGENT_NOTIFICATIONS_FLAG;

/// The longest title, in characters; longer ones are rejected, not cut.
pub(crate) const MAX_TITLE_CHARS: usize = 120;

/// The longest body, in characters; longer ones are rejected, not cut.
pub(crate) const MAX_BODY_CHARS: usize = 1000;

/// The most recipients one notification can name. A sanity bound, far above
/// any real audience.
pub(crate) const MAX_RECIPIENTS: usize = 100;

/// How long an identical notification (same sender, entity, recipients and
/// text) stays a repeat. Windows are fixed spans of the clock, so a repeat is
/// dropped when it falls in the same span as the first.
pub(crate) const DEDUPE_WINDOW: TimeDelta = TimeDelta::minutes(10);

/// Namespace of the ids derived from a manual notification's content.
const NOTIFICATION_ID_NAMESPACE: Uuid = Uuid::from_u128(0x6f1d_2a54_9c3b_4e07_8a51_d2c9_7be4_0f36);

/// The kinds of entity whose view access the shared entity access check
/// answers and the app can open. A link must also be one the inbox lists; see
/// [`shown_in_inbox`]. A channel message is linkable too, and is filed under
/// its channel.
pub(crate) const ACCESS_CHECKED_TYPES: [EntityType; 13] = [
    EntityType::Document,
    EntityType::Chat,
    EntityType::Channel,
    EntityType::Project,
    EntityType::Initiative,
    EntityType::EmailThread,
    EntityType::CalendarEvent,
    EntityType::CrmCompany,
    EntityType::CrmContact,
    EntityType::AgentSession,
    EntityType::ScheduledAction,
    EntityType::Database,
    EntityType::Form,
];

/// Whether the Notifications inbox lists notifications filed under this kind
/// of entity. Anything else would be delivered and then never shown. Home
/// lists the same kinds except companies and contacts, whose notifications
/// have only ever shown in the inbox.
pub(crate) fn shown_in_inbox(entity_type: EntityType) -> bool {
    notification_inbox::domain::policy::CANDIDATE_TYPES.contains(&entity_type)
}

/// Who is sending: the acting user, and the agent whose code runs for them.
pub(crate) struct ManualSender {
    pub user: MacroUserIdStr<'static>,
    pub agent_id: Option<BotId>,
}

/// What the sender wrote and where it points.
pub(crate) struct ManualNotificationDraft {
    /// Who gets it; empty sends it to the sender.
    pub recipients: Vec<MacroUserIdStr<'static>>,
    pub title: String,
    pub body: Option<String>,
    pub link: Entity<'static>,
}

/// Who a manual notification was sent to. A notified recipient's own
/// preferences can still keep it from them; that is theirs to know.
#[derive(Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualNotificationOutcome {
    /// The recipients it was queued for, in the order they were named.
    #[schema(value_type = Vec<String>)]
    pub notified: Vec<MacroUserIdStr<'static>>,
    /// The recipients it was not sent to.
    pub skipped: Vec<SkippedRecipient>,
}

/// A recipient a manual notification was not sent to.
#[derive(Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkippedRecipient {
    /// The recipient.
    #[schema(value_type = String)]
    pub user_id: MacroUserIdStr<'static>,
    /// Why they were left out.
    pub reason: SkipReason,
}

/// Why a recipient was left out of a manual notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// They cannot open the linked entity, so notifying them would leak it.
    NoAccess,
}

/// Why a manual notification was refused.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ManualNotificationError {
    /// The sender's `enable-agent-notifications` flag is off.
    #[error("agent notifications are not enabled for this account")]
    NotEnabled,
    /// The sender's flags could not be read, so nothing says they may send.
    #[error("could not check whether agent notifications are enabled: {0:?}")]
    FlagsUnavailable(rootcause::Report),
    #[error("title must not be empty")]
    EmptyTitle,
    #[error("title is longer than {MAX_TITLE_CHARS} characters")]
    TitleTooLong,
    #[error("body is longer than {MAX_BODY_CHARS} characters")]
    BodyTooLong,
    #[error("a notification can name at most {MAX_RECIPIENTS} recipients")]
    TooManyRecipients,
    #[error("notifications cannot link to a {0}")]
    UnlinkableEntity(EntityType),
    /// The inbox never lists notifications about this kind of entity.
    #[error("notifications linking to a {0} would not appear in the inbox or Home")]
    NotShownInInbox(EntityType),
    /// The inbox and Home leave out unlisted and hidden agent sessions, such
    /// as a routine's runs unless the routine lists them.
    #[error(
        "the linked agent session is not listed, so the notification would not appear in the inbox or Home"
    )]
    UnlistedAgentSession,
    /// The sender cannot see the link, or it does not exist.
    #[error("the linked entity was not found")]
    LinkNotFound,
    /// The sender's allowance has no room for this many recipients.
    #[error(
        "too many notifications: a sender can notify at most {} recipients an hour; try again in {} minutes",
        .0.limit,
        .0.retry_after.as_secs().div_ceil(60)
    )]
    AllowanceExceeded(AllowanceExceeded),
    #[error("internal manual notification error: {0:?}")]
    Internal(rootcause::Report),
}

/// Whether a user can at least view an entity.
pub(crate) trait LinkAccess: Send + Sync + 'static {
    fn can_view(
        &self,
        user: &MacroUserIdStr<'_>,
        entity: &Entity<'_>,
    ) -> impl Future<Output = Result<bool, rootcause::Report>> + Send;
}

/// Whether an agent session is listed: shown wherever its owner's sessions are.
pub(crate) trait SessionListing: Send + Sync + 'static {
    fn is_listed(
        &self,
        session: &Entity<'_>,
    ) -> impl Future<Output = Result<bool, rootcause::Report>> + Send;
}

/// Where a channel message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocatedMessage {
    pub channel_id: Uuid,
    pub message_id: Uuid,
    /// The top-level message it replies to; `None` when it is top-level.
    pub thread_id: Option<Uuid>,
}

/// Finds the channel messages a user can read.
pub(crate) trait ChannelMessages: Send + Sync + 'static {
    /// `None` when there is no such live channel message or `user` cannot
    /// read its channel.
    fn locate(
        &self,
        user: &MacroUserIdStr<'_>,
        message_id: &str,
    ) -> impl Future<Output = Result<Option<LocatedMessage>, rootcause::Report>> + Send;
}

/// A sender's allowance had no room for what they asked to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AllowanceExceeded {
    /// How many notifications the allowance holds per window.
    pub limit: u64,
    /// How long until the allowance is whole again.
    pub retry_after: Duration,
}

/// How many notifications a sender may still send. A ceiling against runaway
/// programs, not a quota ordinary use comes near.
pub(crate) trait SendAllowance: Send + Sync + 'static {
    /// Count `notifications`, one per recipient, against `sender`'s
    /// allowance. Nothing is counted when they do not all fit.
    fn take(
        &self,
        sender: &MacroUserIdStr<'_>,
        notifications: u64,
    ) -> impl Future<Output = Result<Result<(), AllowanceExceeded>, rootcause::Report>> + Send;
}

/// The name and avatar an agent shows as a sender.
pub(crate) trait AgentProfiles: Send + Sync + 'static {
    /// `None` when no such agent exists.
    fn profile(
        &self,
        agent_id: BotId,
    ) -> impl Future<Output = Result<Option<ManualNotificationAgent>, rootcause::Report>> + Send;
}

/// Validates manual notifications and hands them to the notification ingress.
pub(crate) struct ManualNotifications<Flags, Access, Sessions, Messages, Agents, Allowance, Ingress>
{
    pub flags: Flags,
    pub access: Access,
    pub sessions: Sessions,
    pub messages: Messages,
    pub agents: Agents,
    pub allowance: Allowance,
    pub ingress: Ingress,
}

/// The entity a notification is filed under, and the message inside it the
/// sender linked.
struct Target {
    entity: Entity<'static>,
    message: Option<ManualNotificationMessage>,
}

impl<Flags, Access, Sessions, Messages, Agents, Allowance, Ingress>
    ManualNotifications<Flags, Access, Sessions, Messages, Agents, Allowance, Ingress>
where
    Flags: FeatureFlags,
    Access: LinkAccess,
    Sessions: SessionListing,
    Messages: ChannelMessages,
    Agents: AgentProfiles,
    Allowance: SendAllowance,
    Ingress: NotificationIngress,
{
    /// Send `draft` at `now` to each named recipient who can already see its
    /// link. Recipients who cannot are skipped and reported, never given
    /// access. Refused outright while the sender's flag is off.
    #[tracing::instrument(
        name = "notification.send_manual",
        err,
        skip_all,
        fields(
            sender_id = %sender.user,
            agent_id = ?sender.agent_id,
            entity_type = %draft.link.entity_type,
            entity_id = %draft.link.entity_id,
            recipients = draft.recipients.len(),
            notified = tracing::field::Empty,
            skipped = tracing::field::Empty,
        )
    )]
    pub(crate) async fn send(
        &self,
        sender: ManualSender,
        draft: ManualNotificationDraft,
        now: DateTime<Utc>,
    ) -> Result<ManualNotificationOutcome, ManualNotificationError> {
        let enabled = self
            .flags
            .load(&sender.user)
            .await
            .map_err(ManualNotificationError::FlagsUnavailable)?
            .is_enabled(AGENT_NOTIFICATIONS_FLAG);
        if !enabled {
            return Err(ManualNotificationError::NotEnabled);
        }

        let title = draft.title.trim().to_string();
        if title.is_empty() {
            return Err(ManualNotificationError::EmptyTitle);
        }

        if title.chars().count() > MAX_TITLE_CHARS {
            return Err(ManualNotificationError::TitleTooLong);
        }

        let body = draft
            .body
            .map(|body| body.trim().to_string())
            .filter(|body| !body.is_empty());
        if body
            .as_ref()
            .is_some_and(|body| body.chars().count() > MAX_BODY_CHARS)
        {
            return Err(ManualNotificationError::BodyTooLong);
        }

        // Nobody named means the sender; a name given twice counts once.
        let mut named = HashSet::new();
        let mut recipients: Vec<MacroUserIdStr<'static>> = draft
            .recipients
            .into_iter()
            .filter(|recipient| named.insert(recipient.clone()))
            .collect();
        if recipients.len() > MAX_RECIPIENTS {
            return Err(ManualNotificationError::TooManyRecipients);
        }
        if recipients.is_empty() {
            recipients.push(sender.user.clone());
        }

        let target = self.target(&sender.user, draft.link).await?;

        let mut outcome = ManualNotificationOutcome {
            notified: Vec::with_capacity(recipients.len()),
            skipped: Vec::new(),
        };
        for recipient in recipients {
            // The sender's own access was checked with the link.
            let sees_link = recipient == sender.user
                || self
                    .access
                    .can_view(&recipient, &target.entity)
                    .await
                    .map_err(ManualNotificationError::Internal)?;
            if sees_link {
                outcome.notified.push(recipient);
            } else {
                outcome.skipped.push(SkippedRecipient {
                    user_id: recipient,
                    reason: SkipReason::NoAccess,
                });
            }
        }
        let span = tracing::Span::current();
        span.record("notified", outcome.notified.len());
        span.record("skipped", outcome.skipped.len());

        if outcome.notified.is_empty() {
            return Ok(outcome);
        }

        // Counted before sending, so the ceiling holds. A send that then
        // fails stays counted, which errs toward stopping a loop.
        // usize is at most 64 bits on every target this service builds for.
        let notifications = outcome.notified.len() as u64;
        self.allowance
            .take(&sender.user, notifications)
            .await
            .map_err(ManualNotificationError::Internal)?
            .map_err(ManualNotificationError::AllowanceExceeded)?;

        let agent = match sender.agent_id {
            Some(agent_id) => self
                .agents
                .profile(agent_id)
                .await
                .map_err(ManualNotificationError::Internal)?,
            None => None,
        };
        let metadata = ManualMetadata {
            title,
            body,
            agent,
            message: target.message,
        };

        // The ingress drops a recipient who is also the sender, so the
        // sender's own copy goes out separately with no sender.
        let (own, others): (HashSet<_>, HashSet<_>) = outcome
            .notified
            .iter()
            .cloned()
            .partition(|recipient| recipient == &sender.user);
        for (sender_id, recipient_ids) in [(Some(sender.user.clone()), others), (None, own)] {
            if recipient_ids.is_empty() {
                continue;
            }
            // The ingress creates a notification once per id, so a repeat of
            // this one within the window is not delivered again.
            let id = notification_id(&sender, &target.entity, &metadata, &recipient_ids, now);
            let request = SendNotificationRequestBuilder {
                notification_entity: target.entity.clone(),
                secondary_notification_entity: None,
                notification: metadata.clone(),
                sender_id,
                recipient_ids,
            }
            .into_request_with_id(id)
            .with_apns()
            .with_conn_gateway();

            self.ingress
                .send_notification(request)
                .await
                .map_err(|error| ManualNotificationError::Internal(error.into_dynamic()))?;
        }

        Ok(outcome)
    }

    /// What `link` files a notification under, once `sender` is known to see
    /// it. A channel message files it under the message's channel.
    async fn target(
        &self,
        sender: &MacroUserIdStr<'static>,
        link: Entity<'static>,
    ) -> Result<Target, ManualNotificationError> {
        if link.entity_type == EntityType::ChannelMessage {
            let message = self
                .messages
                .locate(sender, &link.entity_id)
                .await
                .map_err(ManualNotificationError::Internal)?
                .ok_or(ManualNotificationError::LinkNotFound)?;
            return Ok(Target {
                entity: EntityType::Channel.with_entity_string(message.channel_id.to_string()),
                message: Some(ManualNotificationMessage {
                    message_id: message.message_id,
                    thread_id: message.thread_id,
                }),
            });
        }

        let entity_type = link.entity_type;
        if !ACCESS_CHECKED_TYPES.contains(&entity_type) {
            return Err(ManualNotificationError::UnlinkableEntity(entity_type));
        }
        if !shown_in_inbox(entity_type) {
            return Err(ManualNotificationError::NotShownInInbox(entity_type));
        }

        let sender_sees_link = self
            .access
            .can_view(sender, &link)
            .await
            .map_err(ManualNotificationError::Internal)?;
        if !sender_sees_link {
            return Err(ManualNotificationError::LinkNotFound);
        }

        // Email threads are left as they are: the inbox shows them only while
        // Signal-important and not shared, which changes after sending.
        if entity_type == EntityType::AgentSession {
            let listed = self
                .sessions
                .is_listed(&link)
                .await
                .map_err(ManualNotificationError::Internal)?;
            if !listed {
                return Err(ManualNotificationError::UnlistedAgentSession);
            }
        }

        Ok(Target {
            entity: link,
            message: None,
        })
    }
}

/// The id of the notification `sender` sends to `recipients`: the same for
/// the same words about the same thing to the same people within one
/// [`DEDUPE_WINDOW`], and different otherwise.
fn notification_id(
    sender: &ManualSender,
    entity: &Entity<'_>,
    metadata: &ManualMetadata,
    recipients: &HashSet<MacroUserIdStr<'static>>,
    now: DateTime<Utc>,
) -> Uuid {
    let mut recipients: Vec<&str> = recipients
        .iter()
        .map(|recipient| recipient.as_ref())
        .collect();
    recipients.sort_unstable();
    let window = now.timestamp().div_euclid(DEDUPE_WINDOW.num_seconds());
    // JSON keeps the fields apart, so no two different notifications share
    // one encoding.
    let content = serde_json::json!([
        sender.user.as_ref(),
        sender.agent_id.map(|agent| agent.as_uuid()),
        entity.entity_type.to_string(),
        entity.entity_id,
        metadata.message,
        metadata.title,
        metadata.body,
        recipients,
        window,
    ]);
    Uuid::new_v5(&NOTIFICATION_ID_NAMESPACE, content.to_string().as_bytes())
}
