import { useEffect, useRef, useState } from "react";
import { api, errorMessage, type FolderAccess, type Settings } from "./api";
import { Sheet, prettyHotkey, row } from "./ui";

export function AccessView({
  settings,
  firstRun,
  onDone,
  setBusy,
}: {
  settings: Settings;
  firstRun: boolean;
  onDone: () => void;
  setBusy: (b: boolean) => void;
}) {
  const [folders, setFolders] = useState<FolderAccess[] | null>(null);
  const [asking, setAsking] = useState(false);
  const [requestError, setRequestError] = useState<string | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [openingSettings, setOpeningSettings] = useState(false);
  const actionRef = useRef<HTMLButtonElement>(null);
  const settingsErrorRef = useRef<HTMLParagraphElement>(null);

  useEffect(() => {
    if (firstRun) return;
    // From Settings: check right away. Granted folders don't prompt again.
    request();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !firstRun) {
        e.preventDefault();
        onDone();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [firstRun, onDone]);

  // Keep keyboard focus on a recovery control when a call fails, instead of
  // leaving it on a button whose label (or whole row) just changed.
  useEffect(() => {
    if (requestError) actionRef.current?.focus();
  }, [requestError]);
  useEffect(() => {
    if (settingsError) settingsErrorRef.current?.focus();
  }, [settingsError]);

  async function request() {
    setAsking(true);
    setRequestError(null);
    setSettingsError(null);
    // macOS's prompts take focus; don't let that close the window.
    setBusy(true);
    try {
      setFolders(await api.requestFolderAccess());
    } catch (e) {
      setRequestError(`Couldn't check folder access: ${errorMessage(e)}`);
    } finally {
      setBusy(false);
      setAsking(false);
    }
  }

  async function openSettings() {
    setSettingsError(null);
    setRequestError(null);
    setOpeningSettings(true);
    setBusy(true);
    try {
      await api.openPrivacySettings();
    } catch (e) {
      setSettingsError(`Couldn't open System Settings: ${errorMessage(e)}`);
    } finally {
      setBusy(false);
      setOpeningSettings(false);
    }
  }

  const missing = folders?.filter((f) => !f.allowed) ?? [];
  const allSet = folders !== null && missing.length === 0;

  return (
    <Sheet
      title={firstRun ? "Welcome to Voxi" : "Folder access"}
      foot={firstRun ? `Press ${prettyHotkey(settings.hotkey)} anywhere to open Voxi.` : "Esc to go back."}
      footRight={
        <button className="link" onClick={onDone}>
          {firstRun && !allSet ? "Skip for now" : "Done"}
        </button>
      }
    >
      <p className="access-lead">
        {firstRun
          ? "Voxi moves, renames, and tidies files when you tell it to. macOS asks once for each folder; allow them now and commands won't stop to ask."
          : "macOS asks once for each folder. Voxi never touches anything outside them."}
      </p>
      <ul className="prefs">
        {(folders ?? DEFAULT_FOLDERS).map((f, i) => (
          <li className="pref access-row" key={f.path} style={row(i)}>
            <span>
              <span className="pref-label">{f.name}</span> <span className="pref-path">{f.path}</span>
            </span>
            <span className={folders === null ? "pref-summary" : f.allowed ? "pref-summary ok" : "pref-summary warn"}>
              {folders === null ? "Not asked yet" : f.allowed ? "Allowed" : "Not allowed"}
            </span>
          </li>
        ))}
      </ul>
      <div className="access-error" role="alert" aria-live="assertive">
        {requestError && <p className="hint warn">{requestError} Choose “Retry” to try again.</p>}
      </div>
      <div className="access-error" role="alert" aria-live="assertive">
        {settingsError && (
          <p ref={settingsErrorRef} tabIndex={-1} className="hint warn">
            {settingsError} Open System Settings yourself from the Apple menu, then go to Privacy &amp; Security → Files and Folders and turn on Voxi.
          </p>
        )}
      </div>
      {missing.length > 0 && (
        <div className="access-help">
          <p className="hint warn">
            Turn on Voxi for {missing.map((f) => f.name).join(", ")} in System Settings → Privacy &amp; Security → Files and Folders → Voxi, then choose “Check again”.
          </p>
          <p className="hint">
            That per-folder grant is all Voxi needs for Desktop, Documents, Downloads and Pictures. Voxi does not need Full Disk Access by default; don't grant it just to get started. Only if you ask Voxi to work in protected locations outside Desktop, Documents or Downloads — such as another app's container — and the folder grant isn't enough, enable Full Disk Access for Voxi under Privacy &amp; Security. Grant it only for those locations, then come back and check again.
          </p>
        </div>
      )}
      <div className="actions">
        {allSet ? (
          <button className="primary-button" autoFocus onClick={onDone}>
            {firstRun ? "Start using Voxi" : "Done"}
          </button>
        ) : (
          <button
            ref={actionRef}
            className="primary-button"
            autoFocus={firstRun && folders === null && !requestError}
            disabled={asking}
            onClick={request}
            aria-label={requestError ? "Retry folder access request" : asking ? "Waiting for macOS" : folders === null ? "Allow folder access" : "Check folder access again"}
          >
            {asking ? "Waiting for macOS…" : requestError ? "Retry" : folders === null ? "Allow access" : "Check again"}
          </button>
        )}
        {missing.length > 0 && (
          <button className="secondary" disabled={openingSettings} onClick={openSettings}>
            {openingSettings ? "Opening System Settings…" : "Open System Settings"}
          </button>
        )}
      </div>
    </Sheet>
  );
}

const DEFAULT_FOLDERS: FolderAccess[] = ["Desktop", "Downloads", "Documents", "Pictures"].map((name) => ({
  name,
  path: `~/${name}`,
  allowed: false,
}));
