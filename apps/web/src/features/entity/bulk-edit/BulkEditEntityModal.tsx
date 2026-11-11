import { isMobile } from '@core/mobile/isMobile';
import { createControlledOpenSignal } from '@core/util/createControlledOpenSignal';
import type { EntityData } from '@entity';
import { ActionDialogShell, Dialog } from '@ui';
import {
  type Accessor,
  createSignal,
  type ParentComponent,
  type Setter,
  Show,
} from 'solid-js';
import { BulkDeleteView, type PartialDeleteHandler } from './BulkDeleteView';
import {
  BulkMoveToProjectView,
  type MovedFolder,
} from './BulkMoveToProjectView';
import { BulkRenameEntitiesView } from './BulkRenameEntitiesView';

const BulkEditEntityModalContent = (props: {
  isOpen: Accessor<boolean>;
  setIsOpen: Setter<boolean>;
  view: 'rename' | 'moveToProject' | 'delete' | null;
  entities: EntityData[];
  onFinish?: () => void;
  onMoved?: (folder: MovedFolder) => void;
  onCancel?: () => void;
  onError?: (error: unknown) => void;
  onPartialDelete?: PartialDeleteHandler;
}) => {
  const handleFinish = () => {
    props.setIsOpen(false);
    props.onFinish?.();
  };
  const handleMoved = (folder: MovedFolder) => {
    const { onMoved } = props;
    handleFinish();
    onMoved?.(folder);
  };
  const handleCancel = () => {
    props.setIsOpen(false);
    props.onCancel?.();
  };
  const handleError = (error: unknown) => {
    props.onError?.(error);
  };

  // The item chips come before the footer, so Kobalte's default autofocus
  // would land on the first chip and Enter would open that item. Focus the
  // Delete button instead so Enter confirms; Escape still cancels. Phone
  // sheets keep focusing themselves, so a screen reader starts on the sheet.
  let deleteButton: HTMLButtonElement | undefined;
  const focusDeleteButton = (event: Event) => {
    if (props.view !== 'delete' || !deleteButton || isMobile()) return;
    event.preventDefault();
    deleteButton.focus();
  };

  return (
    <Dialog
      open={props.isOpen()}
      onOpenAutoFocus={focusDeleteButton}
      position={props.view === 'moveToProject' ? 'top' : 'center'}
      class={props.view === 'moveToProject' ? 'w-140' : 'w-110'}
      onOpenChange={(open) => {
        if (!open) {
          handleCancel();
        }
        props.setIsOpen(open);
      }}
    >
      <Show
        when={props.view === 'moveToProject'}
        fallback={
          <ActionDialogShell>
            <Show when={props.view === 'rename'}>
              <BulkRenameEntitiesView
                entities={props.entities}
                onFinish={handleFinish}
                onCancel={handleCancel}
                onError={handleError}
              />
            </Show>
            <Show when={props.view === 'delete'}>
              <BulkDeleteView
                deleteButtonRef={(el) => {
                  deleteButton = el;
                }}
                onPartialDelete={props.onPartialDelete}
                entities={props.entities}
                onFinish={handleFinish}
                onCancel={handleCancel}
                onError={handleError}
              />
            </Show>
          </ActionDialogShell>
        }
      >
        <BulkMoveToProjectView
          entities={props.entities}
          onMoved={handleMoved}
          onCancel={handleCancel}
          onError={handleError}
        />
      </Show>
    </Dialog>
  );
};

type BulkEditEntityModalProps = {
  isOpen: Accessor<boolean>;
  setIsOpen: Setter<boolean>;
  view: 'rename' | 'moveToProject' | 'delete';
  entities: Accessor<EntityData[]>;
};

const _BulkEditEntityModal: ParentComponent<BulkEditEntityModalProps> = (
  props
) => {
  return (
    <Show when={props.isOpen()}>
      <BulkEditEntityModalContent
        isOpen={props.isOpen}
        setIsOpen={props.setIsOpen}
        view={props.view}
        entities={props.entities()}
      />
    </Show>
  );
};

const [globalModalProps, setGlobalModalProps] = createSignal<{
  view: 'rename' | 'moveToProject' | 'delete';
  entities: EntityData[];
  onFinish?: () => void;
  onMoved?: (folder: MovedFolder) => void;
  onCancel?: () => void;
  onError?: (error: unknown) => void;
  onPartialDelete?: PartialDeleteHandler;
} | null>(null);
const [modalOpen, setModalOpen] = createControlledOpenSignal(false, {
  id: 'entity-edit',
});

export const openBulkEditModal = (props: {
  view: 'rename' | 'moveToProject' | 'delete';
  entities: EntityData[];
  onFinish?: () => void;
  onMoved?: (folder: MovedFolder) => void;
  onCancel?: () => void;
  onError?: (error: unknown) => void;
  onPartialDelete?: PartialDeleteHandler;
}) => {
  setModalOpen(true);
  setGlobalModalProps(props);
};

export const GlobalBulkEditEntityModal = () => {
  const modalProps = () => globalModalProps();

  const handleFinish = () => {
    const props = globalModalProps();
    setGlobalModalProps(null);
    if (props?.onFinish) {
      props.onFinish();
    }
  };

  const handleCancel = () => {
    const props = globalModalProps();
    setGlobalModalProps(null);
    if (props?.onCancel) {
      props.onCancel();
    }
  };

  const handleError = (error: unknown) => {
    globalModalProps()?.onError?.(error);
  };

  return (
    <Show when={modalProps()}>
      {(props) => (
        <BulkEditEntityModalContent
          isOpen={modalOpen}
          setIsOpen={setModalOpen}
          view={props().view}
          entities={props().entities}
          onFinish={handleFinish}
          onMoved={props().onMoved}
          onCancel={handleCancel}
          onError={handleError}
          onPartialDelete={props().onPartialDelete}
        />
      )}
    </Show>
  );
};
