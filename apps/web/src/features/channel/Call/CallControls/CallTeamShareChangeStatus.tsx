import { t } from '@app/lib/i18n';
import { getDisplayName } from '@core/user/displayName';
import { tryMacroId } from '@core/user/macroId';
import { Button } from '@ui';
import { Match, Show, Switch } from 'solid-js';
import type { CallTeamShareChangeControl } from '../use-call-team-share-change';

/**
 * Where a call's team-sharing change stands, with this viewer's answer or
 * their way of withdrawing it.
 */
export function CallTeamShareChangeStatus(props: {
  change: Pick<
    CallTeamShareChangeControl,
    'pending' | 'awaitsMyAnswer' | 'isMine' | 'isAnswering'
  > & { answer: (approve: boolean) => unknown };
}) {
  const waitingOn = () => props.change.pending()?.waitingOn ?? [];
  const waitingLabel = () => {
    const [first, ...rest] = waitingOn();
    if (!first) return t('calls.teamShareSaving');
    if (rest.length === 0)
      return t('calls.teamShareWaitingOne', {
        name: getDisplayName(tryMacroId(first)),
      });
    return t('calls.teamShareWaitingMany', { count: waitingOn().length });
  };
  const requester = () =>
    getDisplayName(tryMacroId(props.change.pending()?.requestedBy ?? ''));
  const shared = () => props.change.pending()?.shared;

  return (
    <Show when={props.change.pending()}>
      <div class="flex flex-col gap-2 px-2 py-1.5 text-xs text-ink-muted">
        <Switch>
          <Match when={props.change.awaitsMyAnswer()}>
            <span>
              {t(
                shared()
                  ? 'calls.teamShareAskShare'
                  : 'calls.teamShareAskPrivate',
                { name: requester() }
              )}
            </span>
            <div class="flex gap-2">
              <Button
                size="sm"
                variant="primary"
                disabled={props.change.isAnswering()}
                onClick={() => props.change.answer(true)}
              >
                {t('calls.teamShareApprove')}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                disabled={props.change.isAnswering()}
                onClick={() => props.change.answer(false)}
              >
                {t('calls.teamShareDecline')}
              </Button>
            </div>
          </Match>
          <Match when={props.change.isMine()}>
            <span>{waitingLabel()}</span>
            <div class="flex gap-2">
              <Button
                size="sm"
                variant="secondary"
                disabled={props.change.isAnswering()}
                onClick={() => props.change.answer(false)}
              >
                {t('calls.teamShareCancel')}
              </Button>
            </div>
          </Match>
          <Match when={true}>
            <span>
              {t(
                shared()
                  ? 'calls.teamShareAskedShare'
                  : 'calls.teamShareAskedPrivate',
                { name: requester() }
              )}
              . {waitingLabel()}.
            </span>
          </Match>
        </Switch>
      </div>
    </Show>
  );
}
