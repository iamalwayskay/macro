import type {
  GetNotificationTypePreferencesResponses,
  ManualNotificationLink,
  NotifEvent,
  SkipReason,
} from '../../../generated/notification/types.gen';
import { unwrap } from '../../utils';
import type { MacroClient } from '../../utils/client';
import { AgentSession } from '../agent-sessions/agent-session';
import { CalendarEvent } from '../calendar/event';
import { Channel } from '../channels/channel';
import { Message } from '../channels/message';
import { Chat } from '../chats/chat';
import { Company } from '../crm/company';
import { Contact } from '../crm/contact';
import { Document } from '../documents/document';
import { EmailThread } from '../email/thread';
import { Initiative } from '../initiatives/initiative';
import { Project } from '../projects/project';
import { User } from '../users/user';
import { Notification } from './notification';

/** A notification event type name, e.g. `channel_mention`. */
export type NotificationType = NotifEvent['tag'];

type NotificationTypePreferences = GetNotificationTypePreferencesResponses[200];

/**
 * What a sent notification is about and opens: the kinds the Notifications
 * inbox lists. Tasks are documents; a `Project` is a folder and an
 * `Initiative` a Tasks project. A channel `Message` opens its channel at
 * that message. An agent session must be listed (a routine's runs are not,
 * unless the routine lists them); an email thread shows only while it is
 * Signal-important and not shared.
 */
export type NotificationLink =
  | Document
  | Channel
  | Message
  | Chat
  | Project
  | Initiative
  | EmailThread
  | CalendarEvent
  | Company
  | Contact
  | AgentSession;

/** A notification to send with {@link NotificationNamespace.send}. */
export interface SendNotificationOptions {
  /** The headline, at most 120 characters; shown as the notification's preview. */
  title: string;
  /** More detail, at most 1000 characters. */
  body?: string;
  /** What it is about and opens; each recipient must already be able to open it. */
  link: NotificationLink;
  /** Who gets it, at most 100 people; you when omitted. */
  to?: User | User[];
}

/** A recipient {@link NotificationNamespace.send} left out. */
export interface SkippedNotificationRecipient {
  /** Who was left out. */
  user: User;
  /** Why: `no_access` means they cannot open `link`. */
  reason: SkipReason;
}

/** Who {@link NotificationNamespace.send} notified. */
export interface SendNotificationResult {
  /** The recipients it was sent to. */
  notified: User[];
  /** The recipients it was not sent to. */
  skipped: SkippedNotificationRecipient[];
}

function linkReference(link: NotificationLink): ManualNotificationLink {
  return { entityType: linkEntityType(link), entityId: link.id };
}

// Handles have protected constructors, which `P.instanceOf` cannot take.
function linkEntityType(
  link: NotificationLink,
): ManualNotificationLink['entityType'] {
  if (link instanceof Document) return 'document';
  if (link instanceof Channel) return 'channel';
  if (link instanceof Message) return 'channel_message';
  if (link instanceof Chat) return 'chat';
  if (link instanceof Project) return 'project';
  if (link instanceof Initiative) return 'initiative';
  if (link instanceof EmailThread) return 'email_thread';
  if (link instanceof CalendarEvent) return 'calendar_event';
  if (link instanceof Company) return 'crm_company';
  if (link instanceof Contact) return 'crm_contact';
  if (link instanceof AgentSession) return 'agent_session';
  throw new TypeError(
    'notifications.send: link must be a document, task, channel, channel message, chat, folder, project, email thread, calendar event, company, contact or agent session',
  );
}

/**
 * @routing Use when reading the user's notifications or marking them seen, or when notifying the user or other people about a task, document, channel message, email, project, event or CRM record; not sending messages.
 */
export class NotificationNamespace {
  constructor(private readonly client: MacroClient) {}

  /** A handle to a notification by id. */
  byId(id: string): Notification {
    return Notification.byId(this.client, id);
  }

  /** The viewer's notification feed, most recent first, auto-paginated. */
  list(opts?: { pageSize?: number }): AsyncGenerator<Notification> {
    return Notification.list(this.client, opts);
  }

  /**
   * Notify yourself or other people about `link`, which each must already be able to open; post a message instead when a reply is expected.
   *
   * The notification appears in each recipient's Notifications on the row of
   * what `link` is (a task's or document's comments, a channel or one of its
   * message threads, an email, a project, a calendar event, a company or
   * contact), with `title` as its preview, and opens `link` when clicked.
   * Routines should use this to report back, linking the document or listed
   * session the run produced; links to routines, databases and forms, and to
   * unlisted or hidden sessions, are refused because the inbox would never
   * show them.
   *
   * Nobody is given access: a recipient who cannot open `link` is returned
   * in `skipped` with reason `no_access` and gets nothing, so share `link`
   * with them first if they should see it. It is delivered like any other
   * notification (in-app, desktop and phone push) and respects each
   * recipient's notification preferences, so someone in `notified` may still
   * have it muted.
   *
   * Do not send in a loop to repeat yourself: the same title and body about
   * the same `link` to the same people is delivered once in each ten-minute
   * window of the clock, so a repeat is dropped unless it falls in the next
   * window.
   * `to` takes at most 100 people, and a sender can notify at most 1,000
   * recipients an hour; past that the call fails until the hour is up.
   *
   * Sending is being rolled out per account. Where it is off, the call fails
   * with a `MacroForbiddenError` saying agent notifications are not enabled:
   * tell the user so, and do not request access to `link`, which is not what
   * is missing.
   *
   * @example
   * ```ts
   * const task = macro.tasks.byId(taskId);
   * const { skipped } = await macro.notifications.send({
   *   title: 'Ready for review',
   *   body: 'The fix is in; tests pass.',
   *   link: task,
   *   to: [reviewer, owner],
   * });
   * ```
   */
  async send(
    options: SendNotificationOptions,
  ): Promise<SendNotificationResult> {
    const recipients = options.to === undefined ? [] : [options.to].flat();
    const outcome = unwrap(
      await this.client.notification.sendManualNotification({
        body: {
          title: options.title,
          body: options.body,
          link: linkReference(options.link),
          recipientIds:
            recipients.length > 0
              ? recipients.map((recipient) => recipient.id)
              : undefined,
        },
      }),
    );
    // Hand back the caller's own handles, which may already hold names.
    const given = new Map(recipients.map((user) => [user.id, user]));
    const handle = (id: string) => given.get(id) ?? User.byId(this.client, id);
    return {
      notified: outcome.notified.map(handle),
      skipped: outcome.skipped.map(({ userId, reason }) => ({
        user: handle(userId),
        reason,
      })),
    };
  }

  /** Mark a batch of notifications as seen. */
  async markSeen(ids: string[]): Promise<void> {
    unwrap(
      await this.client.notification.bulkMarkNotificationsSeen({
        body: { notificationIds: ids },
      }),
    );
  }

  /** Mark a batch of notifications as done. */
  async markDone(ids: string[]): Promise<void> {
    unwrap(
      await this.client.notification.bulkMarkNotificationsDone({
        body: { notificationIds: ids },
      }),
    );
  }

  /** Delete a batch of notifications. */
  async delete(ids: string[]): Promise<void> {
    unwrap(
      await this.client.notification.bulkDeleteUserNotificationsV2({
        body: { notificationIds: ids },
      }),
    );
  }

  /** The viewer's notification type preferences (disabled types). */
  async preferences(): Promise<NotificationTypePreferences> {
    return unwrap(
      await this.client.notification.getNotificationTypePreferences(),
    );
  }

  /** Re-enable delivery of a notification type. */
  async enableType(type: NotificationType): Promise<void> {
    unwrap(
      await this.client.notification.enableNotificationType({
        path: { notification_event_type: type },
      }),
    );
  }

  /** Disable delivery of a notification type. */
  async disableType(type: NotificationType): Promise<void> {
    unwrap(
      await this.client.notification.disableNotificationType({
        path: { notification_event_type: type },
      }),
    );
  }
}
