# notification_service

## What it is

Serves Notifications across the product: a deployable service that composes
the `notification` domain crate into a running process. Exposes an HTTP API
for reading/unsubscribing from notifications, plus the Kafka-driven egress
worker that sends email/mobile-push/websocket notifications.

## Start here

- `src/main.rs`: composition root — wires `NotificationEgressService`,
  `NotificationWorker`, `NotificationEventsListener`, and the outbound adapters
  (`EmailAdapter`, `MobilePushAdapter`, `WebSocketGatewayAdapter`,
  `Kafka*RealtimePublisher`, `RedisRateLimitAdapter`) from `crates/notification`.
- `src/api.rs` + `src/api/context.rs`: HTTP routes — `user_notification.rs`
  (list/read), `unsubscribe.rs` (single and atomic bulk item mute/snooze), `health.rs`.
- `src/manual.rs` + `src/api/manual_notification.rs`: manual notifications
  (`POST /user_notifications/send`, the SDK's `notifications.send`). The
  domain checks that the sender can see the link and that the inbox lists
  its kind (`notification_inbox`'s `CANDIDATE_TYPES`; agent sessions must be
  listed), skips and reports each recipient who cannot see it, snapshots the
  sending agent, and enqueues through the notification ingress. A channel
  message link is read through the message service
  (`src/manual/channel_messages.rs`) and filed under its channel. Its
  notification ids are derived from sender, entity, recipients, text and the
  fixed ten-minute window the send falls in, so the ingress drops a repeat in
  the same window; `src/manual/allowance.rs`
  caps each sender at 1,000 notified recipients an hour. It lives here, not in
  `crates/notification`, because `model_notifications` depends on that crate.
- `src/manual/flags.rs`: where the `enable-agent-notifications` flag is read
  from. `ManualNotifications::send` reads it for the sender before anything
  else.
- `src/model/notification.rs`: service-local request/response shapes.
- Push payload schemas come from `notification::domain::models::apple`; keep the
  frontend push validators generated from those same OpenAPI components.
- `README.md`: local DB setup (`sqlx database setup` from
  `crates/notification_db_client`).

## Scope

- Owns: process wiring (config, adapters, HTTP routes) for notifications; the
  domain logic (digesting, egress rules, send decisions) lives in
  `crates/notification` and `crates/notification_db_client`.
- Not here: notification domain logic itself → `crates/notification`.

## Recipes

- Add a notification HTTP route: `src/api/user_notification.rs` (or a new
  module) → `src/api.rs` → `src/api/swagger.rs`.
- Add a new egress channel: implement the port in `crates/notification`
  (`outbound/`) → wire the adapter in `src/main.rs`.

## Checks

- `cargo test -p notification_service` (`SQLX_OFFLINE` unset; needs local
  MacroDB — `just run_dbs -d`, then `sqlx database setup` from
  `crates/notification_db_client`).

## Gotchas

- `src/manual/test/delivered.rs` sends a manual notification for every
  linkable kind and reads it back through the real inbox and Home services
  over Soup; a kind that becomes linkable needs an entity in
  `fixtures/manual_links.sql`, or the test fails. Calendar events, companies
  and contacts are the exception: Soup lists them through services that
  harness stubs, so the test names them in `NOT_LISTED_HERE`.

- PostgreSQL delete forwarding emits `notification.delete.publish` spans and
  logs publication failures. The retention Lambda is a separate binary/role;
  see `services/notification_retention_handler` and its rollout runbook.

- Don't log notification payloads on tracing spans (#7892) — they can carry
  sensitive content; log ids/types instead.
- Manual notifications are flag-gated per sender. A local stack has no PostHog
  project, so they are on for everyone there; a deployment reads PostHog with
  the optional `POSTHOG_API_KEY` (and `POSTHOG_HOST`), and without the key
  they are off: `/user_notifications/send` answers 403. A flag that cannot be
  read answers 503, never a send.
- `SES_CONFIGURATION_SET` is optional. When set, every notification email is
  sent through that SES configuration set and tagged `notification_type`, so
  its event destination reports delivery, bounce, open and click per type. Set
  it only once the configuration set exists in that account and region and
  the stack's SES policy (which allows `configuration-set/*`) is deployed.

## Related

- Skill: `new-service`

- The HTTP boundary validates optional signed agent correlation without changing existing authorization.
