import { invoke } from "@tauri-apps/api/core";
import { Modal } from "./Modal";

export interface UpdateStatus {
  current: string;
  latest: string;
  available: boolean;
}

export function UpdateNotice({
  status,
  onClose,
  onNever,
}: {
  status: UpdateStatus;
  onClose: () => void;
  onNever: () => void;
}) {
  return (
    <Modal title="Update available" onClose={onClose}>
      <p>
        YuuTerm {status.latest} is available. You have {status.current}.
      </p>
      <p className="hint">
        Nothing is installed automatically. The download page opens in your
        browser.
      </p>
      <div className="dialog-actions">
        <button onClick={onNever}>Don&apos;t remind me again</button>
        <button onClick={onClose}>Later</button>
        <button
          className="primary"
          onClick={() => {
            void invoke("open_release_page").catch(() => {});
            onClose();
          }}
        >
          Open download page
        </button>
      </div>
    </Modal>
  );
}
