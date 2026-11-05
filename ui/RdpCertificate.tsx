import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** Remembered RDP server certificate of a saved session ("Don't ask me again"). */
export function RdpCertificate({
  profileId,
  viaGateway = false,
}: {
  profileId: number;
  viaGateway?: boolean;
}) {
  const [remembered, setRemembered] = useState<boolean | null>(null);
  const [status, setStatus] = useState("");
  const refresh = () =>
    void invoke<boolean>("rdp_certificate_remembered", { profileId })
      .then(setRemembered)
      .catch((e) => setStatus(String(e)));
  useEffect(() => {
    if (profileId) refresh();
  }, [profileId]);
  if (!profileId)
    return (
      <p className="hint">
        {viaGateway ? (
          "Server certificate: review the original target fingerprint in the native prompt; choose Remember and connect to retain it."
        ) : (
          <>
            Server certificate: tick "Don't ask me again" in Windows'
            certificate prompt to connect without it next time.
          </>
        )}
      </p>
    );
  return (
    <div className="rdp-certificate">
      <span>
        Server certificate:{" "}
        {remembered === null
          ? "checking…"
          : remembered
            ? "remembered — connects without the prompt while it stays the same"
            : viaGateway
              ? 'asks on every connection — choose "Remember and connect" in the native prompt'
              : 'asks on every connection — tick "Don\'t ask me again" in the prompt to remember it'}
      </span>
      {remembered && (
        <button
          type="button"
          onClick={() =>
            void invoke("rdp_forget_certificate", { profileId })
              .then(() => {
                setStatus("Forgotten; the next connection asks again.");
                refresh();
              })
              .catch((e) => setStatus(String(e)))
          }
        >
          Forget
        </button>
      )}
      {status && <small role="status">{status}</small>}
    </div>
  );
}
