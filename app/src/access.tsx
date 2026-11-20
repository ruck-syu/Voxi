import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage, type FolderAccess, type Settings } from "./api";
import { Sheet, prettyHotkey, row } from "./ui";

type AccessAction = "checking" | "openingSettings" | "saving";
type AccessState = { k: "idle" } | { k: AccessAction } | { k: "error"; action: AccessAction; message: string };

export function AccessView({
  settings,
  firstRun,
  onDone,
  beginOperation,
}: {
  settings: Settings;
  firstRun: boolean;
  onDone: () => void | Promise<void>;
  beginOperation: () => () => void;
}) {
  const [folders, setFolders] = useState<FolderAccess[] | null>(null);
  const [state, setState] = useState<AccessState>({ k: "idle" });
  const owner = useRef<symbol | null>(null);
  const actionRef = useRef<HTMLButtonElement>(null);
  const errorRef = useRef<HTMLParagraphElement>(null);
  const busy = state.k !== "idle" && state.k !== "error";
  const asking = state.k === "checking";
  const requestError = state.k === "error" && state.action === "checking" ? state.message : null;
  const settingsError = state.k === "error" && state.action === "openingSettings" ? state.message : null;
  const saveError = state.k === "error" && state.action === "saving" ? state.message : null;

  const run = useCallback(async (action: AccessAction, task: () => void | Promise<void>, errorPrefix: string) => {
    if (owner.current !== null) return;
    const operation = Symbol();
    owner.current = operation;
    const release = beginOperation();
    setState({ k: action });
    try {
      await task();
      if (owner.current === operation) setState({ k: "idle" });
    } catch (e) {
      if (owner.current === operation) setState({ k: "error", action, message: `${errorPrefix}: ${errorMessage(e)}` });
    } finally {
      if (owner.current === operation) owner.current = null;
      release();
    }
  }, [beginOperation]);

  const request = useCallback(() => run("checking", async () => {
    setFolders(await api.requestFolderAccess());
  }, "Couldn't check folder access"), [run]);
  const openSettings = () => run("openingSettings", api.openPrivacySettings, "Couldn't open System Settings");
  const done = useCallback(() => run("saving", onDone, "Couldn't save onboarding"), [run, onDone]);

  useEffect(() => {
    if (firstRun) return;
    // From Settings: check right away. Granted folders don't prompt again.
    void request();
  }, [firstRun, request]);

  useEffect(() => {
    if (firstRun) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        void done();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [firstRun, done]);

  useEffect(() => {
    if (requestError) actionRef.current?.focus();
    if (settingsError || saveError) errorRef.current?.focus();
  }, [requestError, settingsError, saveError]);

  const missing = folders?.filter((f) => !f.allowed) ?? [];
  const allSet = folders !== null && missing.length === 0;

  return (
    <Sheet
      title={firstRun ? "Welcome to Voxi" : "Folder access"}
      foot={firstRun
        ? settings.hotkey ? `Press ${prettyHotkey(settings.hotkey)} anywhere to open Voxi.` : "Open Voxi from the menu bar."
        : "Esc to go back."}
      footRight={
        <button className="link" disabled={busy} onClick={done}>
          {state.k === "saving" ? "Saving…" : firstRun && !allSet ? "Skip for now" : "Done"}
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
        {(settingsError || saveError) && (
          <p ref={errorRef} tabIndex={-1} className="hint warn">
            {saveError || `${settingsError} Open System Settings yourself from the Apple menu, then go to Privacy & Security → Files and Folders and turn on Voxi.`}
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
          <button className="primary-button" autoFocus disabled={busy} onClick={done}>
            {firstRun ? "Start using Voxi" : "Done"}
          </button>
        ) : (
          <button
            ref={actionRef}
            className="primary-button"
            autoFocus={firstRun && folders === null && !requestError}
            disabled={busy}
            onClick={request}
            aria-label={requestError ? "Retry folder access request" : asking ? "Waiting for macOS" : folders === null ? "Allow folder access" : "Check folder access again"}
          >
            {asking ? "Waiting for macOS…" : requestError ? "Retry" : folders === null ? "Allow access" : "Check again"}
          </button>
        )}
        {missing.length > 0 && (
          <button className="secondary" disabled={busy} onClick={openSettings}>
            {state.k === "openingSettings" ? "Opening System Settings…" : "Open System Settings"}
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
