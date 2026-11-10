import { describe, expect, it } from 'vitest';
import {
  listedNotificationEvents,
  NOTIFICATION_EVENT_GROUPS,
} from '../notification-event-catalog';

const labels = (agentNotifications: boolean) =>
  NOTIFICATION_EVENT_GROUPS.flatMap((group) =>
    listedNotificationEvents(group, { agentNotifications }).map(
      (event) => event.label
    )
  );

describe('the notification settings a person is offered', () => {
  it('lists Agent notes while agent notifications are on for them', () => {
    expect(labels(true)).toContain('Agent notes');
  });

  it('leaves out only Agent notes while they are off', () => {
    expect(labels(false)).toEqual(
      labels(true).filter((label) => label !== 'Agent notes')
    );
    expect(labels(false)).toContain('Routine runs');
  });
});
