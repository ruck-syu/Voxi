import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { api, errorMessage, tildify, type Reports, type ScheduledTask, type Settings, type Shortcut } from "./api";
import { appVersion, checkAndInstall, type UpdateState } from "./updates";
import { Sheet, TOO_LONG, prettyHotkey, shortcutLengthIssue } from "./ui";

const KEY_NAMES: Record<string, string> = { " ": "Space", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right" };

function comboFromEvent(e: React.KeyboardEvent): string | null {
  if (["Meta", "Control", "Alt", "Shift"].includes(e.key)) return null;
  const mods = [e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
  if (mods.length === 0) return null;
  let key = e.code.startsWith("Key") ? e.code.slice(3) : e.code.startsWith("Digit") ? e.code.slice(5) : e.code;
  key = KEY_NAMES[e.key] ?? key;
  return [...mods, key].join("+");
}

/** Everything a Settings tab can use. */
type TabContext = {
  settings: Settings;
  persist: (s: Settings) => Promise<boolean>;
  setBusy: (b: boolean) => void;
  activeTasks: number | null;
};

/** 16px line icons, drawn in the current text color. */
const ICONS = {
  general: <><circle cx="8" cy="8" r="2.2" /><path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4" /></>,
  scheduled: <><circle cx="8" cy="8" r="6" /><path d="M8 4.5V8l2.5 1.5" /></>,
  shortcuts: <path d="M9 1.5 3.5 9H8l-1 5.5L12.5 7H8z" />,
  folders: <path d="M1.75 4.25c0-.7.55-1.25 1.25-1.25h3l1.5 1.5H13c.7 0 1.25.55 1.25 1.25v6c0 .7-.55 1.25-1.25 1.25H3c-.7 0-1.25-.55-1.25-1.25z" />,
  keyboard: <><rect x="1.5" y="4" width="13" height="8" rx="1.5" /><path d="M4 7h.01M6.5 7h.01M9 7h.01M11.5 7h.01M5 9.5h6" /></>,
  power: <><path d="M8 1.75V7.5" /><path d="M4.6 3.6a5.25 5.25 0 1 0 6.8 0" /></>,
  mic: <><rect x="5.75" y="1.5" width="4.5" height="8" rx="2.25" /><path d="M3.5 7.5a4.5 4.5 0 0 0 9 0M8 12v2.5" /></>,
  update: <><path d="M13.5 8A5.5 5.5 0 1 1 11.9 4.1" /><path d="M13.5 2v3h-3" /></>,
  shield: <path d="M8 1.75 2.75 3.75V7.5c0 3.1 2.2 5.6 5.25 6.75 3.05-1.15 5.25-3.65 5.25-6.75V3.75z" />,
  tag: <><path d="M1.75 8.4V2.75c0-.55.45-1 1-1H8.4l5.85 5.85-6.65 6.65z" /><circle cx="5" cy="5" r="1" /></>,
} as const;

function Icon({ name }: { name: keyof typeof ICONS }) {
  return <svg className="icon" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{ICONS[name]}</svg>;
}

/** Settings sections, in sidebar order. Add a tab by adding an entry here. */
const TABS = [
  {
    id: "general", label: "General", icon: "general",
    badge: () => null,
    render: (c: TabContext) => <GeneralTab settings={c.settings} persist={c.persist} />,
  },
  {
    id: "scheduled", label: "Scheduled", icon: "scheduled",
    badge: (c: TabContext) => c.activeTasks || null,
    render: () => <ScheduledTab />,
  },
  {
    id: "shortcuts", label: "Shortcuts", icon: "shortcuts",
    badge: (c: TabContext) => c.settings.shortcuts.length || null,
    render: (c: TabContext) => <ShortcutsTab settings={c.settings} persist={c.persist} />,
  },
  {
    id: "folders", label: "Folders", icon: "folders",
    badge: () => null,
    render: (c: TabContext) => <FoldersTab settings={c.settings} persist={c.persist} setBusy={c.setBusy} />,
  },
] as const satisfies readonly {
  id: string; label: string; icon: keyof typeof ICONS;
  badge: (c: TabContext) => number | null; render: (c: TabContext) => React.ReactNode;
}[];
type Tab = (typeof TABS)[number]["id"];

/** A titled group of rows on a soft card, like System Settings. */
function Group({ title, action, foot, children }: { title: string; action?: React.ReactNode; foot?: React.ReactNode; children: React.ReactNode }) {
  return <section className="group">
    <div className="group-head">
      <h3 className="group-title">{title}</h3>
      {action}
    </div>
    <ul className="group-rows">{children}</ul>
    {foot && <p className="group-foot">{foot}</p>}
  </section>;
}

/** One setting: an optional icon, a label with a short description, and its control. */
function Row({ icon, label, htmlFor, description, children, className }: {
  icon?: keyof typeof ICONS; label?: React.ReactNode; htmlFor?: string; description?: React.ReactNode;
  children?: React.ReactNode; className?: string;
}) {
  return <li className={className ? `row ${className}` : "row"}>
    {icon && <span className="row-icon"><Icon name={icon} /></span>}
    {label !== undefined && <div className="row-copy">
      {htmlFor ? <label className="row-label" htmlFor={htmlFor}>{label}</label> : <span className="row-label">{label}</span>}
      {description && <span className="row-desc">{description}</span>}
    </div>}
    {children}
  </li>;
}

const ACTIVE_STATUSES: ScheduledTask["status"][] = ["pending", "running", "paused"];

function RecoveryNotice() {
  const [reviewed, setReviewed] = useState(false);
  const [recovery, setRecovery] = useState<Awaited<ReturnType<typeof api.recoveryStatus>>>(null);
  const [error, setError] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const actionRef = useRef<HTMLButtonElement>(null);
  const messageRef = useRef<HTMLParagraphElement>(null);
  const generation = useRef(0);
  useEffect(() => {
    let alive = true;
    const refresh = () => {
      const started = generation.current;
      return api.recoveryStatus().then((next) => {
        if (alive && started === generation.current) { setRecovery(next); setError(null); }
      }).catch((e) => { if (alive && started === generation.current) setError(errorMessage(e)); });
    };
    void refresh();
    const timer = window.setInterval(refresh, 2000);
    return () => { alive = false; clearInterval(timer); };
  }, []);
  const review = async () => {
    if (working) return;
    setWorking(true);
    try {
      await api.acknowledgeRecovery();
      ++generation.current; // ignore polls started before the acknowledgement
      setRecovery(null); setError(null); setReviewed(true);
      requestAnimationFrame(() => messageRef.current?.focus());
    } catch (e) {
      setError(errorMessage(e));
      actionRef.current?.focus();
    } finally { setWorking(false); }
  };
  return <div className="settings-note recovery-notice">
    <p ref={messageRef} tabIndex={-1} role={recovery || error ? "alert" : "status"}>
      {recovery ? `Interrupted operation “${recovery.command}”. ${recovery.acknowledged} steps acknowledged. Check the files below, then choose Mark as reviewed in Settings → Scheduled to allow new changes. No action was retried.` : error ? "Recovery status unavailable." : reviewed ? "Recovery evidence archived. New changes are allowed." : ""}
      {error && ` ${error}`}
    </p>
    {recovery && <>
      {recovery.inFlight && <pre>{JSON.stringify(recovery.inFlight, null, 2)}</pre>}
      <button ref={actionRef} className="link" aria-disabled={working} onClick={() => void review()}>{working ? "Archiving…" : "Mark as reviewed"}</button>
      <p className="hint">The recovery journal will be archived beside the original. Review does not retry or undo the interrupted step.</p>
    </>}
  </div>;
}

function ScheduledTab() {
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [working, setWorking] = useState<number | null>(null);
  const [stoppingRun, setStoppingRun] = useState<string | null>(null);
  const [showing, setShowing] = useState<"active" | "done">("active");
  useEffect(() => {
    let alive = true;
    const refresh = () => Promise.all([api.scheduledTasks(), api.schedulerNotice()]).then(([next, message]) => {
      if (alive) { setTasks(next); setNotice(message); setLoading(false); setError(null); }
    }).catch((e) => { if (alive) { setError(errorMessage(e)); setLoading(false); } });
    void refresh();
    const timer = window.setInterval(refresh, 2000);
    return () => { alive = false; clearInterval(timer); };
  }, []);
  const act = async (id: number, action: (id: number) => Promise<void>) => {
    setWorking(id);
    try { await action(id); setTasks(await api.scheduledTasks()); setError(null); }
    catch (e) { setError(errorMessage(e)); }
    finally { setWorking(null); }
  };
  const stopRun = async (task: ScheduledTask, run: string) => {
    setStoppingRun(run);
    try {
      await api.stopExecution(task.id, task.due);
    } catch (e) {
      setStoppingRun((current) => current === run ? null : current);
      setError(errorMessage(e));
    }
  };
  const when = (task: ScheduledTask) => {
    const at = new Date(task.due).toLocaleString(undefined, { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
    if (!task.repeat) return at;
    const every = `${task.repeat[0].toUpperCase()}${task.repeat.slice(1)}`;
    return task.status === "pending" ? `${every} · next ${at}` : every;
  };
  const STATUS: Record<ScheduledTask["status"], [string, string]> = {
    pending: ["Scheduled", "ok"], running: ["Running", "ok"], paused: ["Paused", "muted"],
    completed: ["Done", "muted"], cancelled: ["Stopped", "muted"], failed: ["Failed", "warn"],
  };
  const active = tasks.filter((t) => ACTIVE_STATUSES.includes(t.status)).reverse();
  const done = tasks.filter((t) => !ACTIVE_STATUSES.includes(t.status)).reverse();
  const shown = showing === "active" ? active : done;
  const button = (task: ScheduledTask, label: string, busy: string, action: (id: number) => Promise<void>) =>
    <button className="link" disabled={working !== null} onClick={() => void act(task.id, action)}>{working === task.id ? busy : label}</button>;
  return <>
    <div className="segments" role="group" aria-label="Show tasks">
      {([["active", "Active", active.length], ["done", "Done", done.length]] as const).map(([id, label, count]) =>
        <button key={id} aria-pressed={showing === id} onClick={() => setShowing(id)}>{label} <span className="count">{count}</span></button>)}
    </div>
    <RecoveryNotice />
    {notice && <p className="settings-note warn" role="status">{notice}</p>}
    {error && <p className="settings-note warn" role="alert">{error}</p>}
    {loading ? <p className="quiet">Loading scheduled tasks…</p> : shown.length === 0 ? <div className="empty">
      <span className="empty-icon"><Icon name="scheduled" /></span>
      {showing === "active" ? <>
        <p className="empty-title">Nothing scheduled</p>
        <p className="empty-desc">End a command with a time to run it later.</p>
        <p className="empty-examples"><kbd>at 9 pm</kbd><kbd>in 3 hours</kbd><kbd>every 3 days</kbd></p>
      </> : <>
        <p className="empty-title">No finished tasks yet</p>
        <p className="empty-desc">Tasks that ran or were stopped show up here.</p>
      </>}
    </div> :
      <ul className="group-rows tasks-list">{shown.map((task) => {
        const [status, tone] = STATUS[task.status];
        const run = `${task.id}-${task.due}`;
        return <li className="row task" key={task.id}>
          <span className="row-icon"><Icon name={task.repeat ? "update" : "scheduled"} /></span>
          <div className="row-copy">
            <span className="row-label task-command">{task.command}</span>
            <span className="row-desc">{when(task)}</span>
            {task.summary && <span className="row-desc task-summary">{task.summary}</span>}
            <span className="task-actions">
              {task.status === "running" && <button type="button" className="link" disabled={stoppingRun === run} onClick={() => void stopRun(task, run)}>
                {stoppingRun === run ? "Stopping…" : "Stop this run"}
              </button>}
              {task.repeat && task.status === "pending" && button(task, "Pause", "Pausing…", api.pauseScheduledTask)}
              {task.status === "paused" && button(task, "Resume", "Resuming…", api.resumeScheduledTask)}
              {(task.status === "pending" || task.status === "paused") && button(task, task.repeat ? "Stop repeating" : "Cancel task", "Stopping…", api.cancelScheduledTask)}
              {task.repeat && (task.status === "cancelled" || task.status === "failed") && button(task, "Move to active", "Moving…", api.resumeScheduledTask)}
            </span>
          </div>
          <span className={`pill ${tone}`}>{status}</span>
        </li>;
      })}</ul>}
  </>;
}

/** The Settings tab bar, on the title row to the right of "Settings". */
function SettingsTabs({ value, saving, context, onSelect }: { value: Tab; saving: boolean; context: TabContext; onSelect: (tab: Tab) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [indicator, setIndicator] = useState<{ left: number; width: number } | null>(null);
  const [animate, setAnimate] = useState(false);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => {
      const selected = element.querySelector<HTMLButtonElement>('[aria-selected="true"]');
      if (selected) setIndicator({ left: selected.offsetLeft, width: selected.offsetWidth });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    const frame = requestAnimationFrame(() => setAnimate(true));
    return () => { observer.disconnect(); cancelAnimationFrame(frame); };
  }, [value]);
  return <div className="tabs" role="tablist" aria-label="Settings sections" ref={ref}>
    {indicator && <span className="tab-highlight" data-animate={animate || undefined} aria-hidden="true" style={indicator} />}
    {TABS.map((tab, index) => {
      const badge = tab.badge(context);
      return <button key={tab.id} role="tab" id={`settings-tab-${tab.id}`}
        aria-selected={value === tab.id} aria-disabled={saving} aria-controls="settings-panel"
        tabIndex={value === tab.id ? 0 : -1} onClick={() => onSelect(tab.id)}
        onKeyDown={(event) => {
          if (saving) return;
          const next = event.key === "ArrowRight" ? (index + 1) % TABS.length
            : event.key === "ArrowLeft" ? (index + TABS.length - 1) % TABS.length
            : event.key === "Home" ? 0 : event.key === "End" ? TABS.length - 1 : null;
          if (next === null) return;
          event.preventDefault();
          onSelect(TABS[next].id);
          ref.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
        }}>
        <Icon name={tab.icon} />
        <span className="tab-label">{tab.label}</span>
        {badge !== null && <span className="tab-badge" aria-label={`${badge}`}>{badge}</span>}
      </button>;
    })}
  </div>;
}

/** Settings save as they change: no Save button. */
export function SettingsView({
  settings,
  onSaved,
  onClose,
  setBusy,
}: {
  settings: Settings;
  onSaved: (s: Settings) => void;
  onClose: () => void;
  setBusy: (b: boolean) => void;
}) {
  const [tab, setTab] = useState<Tab>("general");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [activeTasks, setActiveTasks] = useState<number | null>(null);
  const savePending = useRef(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !savePending.current) {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // The sidebar's count of active scheduled tasks.
  useEffect(() => {
    let alive = true;
    const refresh = () => api.scheduledTasks()
      .then((tasks) => { if (alive) setActiveTasks(tasks.filter((t) => ACTIVE_STATUSES.includes(t.status)).length); })
      .catch(() => {});
    void refresh();
    const timer = window.setInterval(refresh, 4000);
    return () => { alive = false; clearInterval(timer); };
  }, []);

  const persist = async (next: Settings): Promise<boolean> => {
    if (savePending.current) return false;
    savePending.current = true;
    const focused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setSaving(true);
    try {
      onSaved(await api.saveSettings(next));
      setError(null);
      return true;
    } catch (e) {
      setError(errorMessage(e));
      return false;
    } finally {
      savePending.current = false;
      setSaving(false);
      requestAnimationFrame(() => {
        if (focused?.isConnected && document.activeElement === document.body && !focused.matches(":disabled")) focused.focus();
      });
    }
  };

  const context: TabContext = { settings, persist, setBusy, activeTasks };
  const current = TABS.find((t) => t.id === tab)!;

  return (
    <Sheet
      title="Settings"
      aside={<SettingsTabs value={tab} saving={saving} context={context} onSelect={(id) => { if (!savePending.current) setTab(id); }} />}
      bodyClass="settings-body"
      foot={error ? <span className="warn" role="alert">{error}</span> : "Esc to go back."}
      footRight={
        <button className="link" aria-disabled={saving} onClick={() => { if (!savePending.current) onClose(); }}>
          Done
        </button>
      }
    >
      <fieldset key={tab} id="settings-panel" role="tabpanel" tabIndex={0} aria-labelledby={`settings-tab-${tab}`} className="tab-panel" disabled={saving} aria-busy={saving}>
        {current.render(context)}
      </fieldset>
    </Sheet>
  );
}

type TabProps = { settings: Settings; persist: (s: Settings) => Promise<boolean> };

function GeneralTab({ settings, persist }: TabProps) {
  const [recording, setRecording] = useState(false);
  const [linkError, setLinkError] = useState<string | null>(null);
  return <>
    <Group title="Startup">
      <Row icon="keyboard" label="Open Voxi" description={recording ? "Press the new keys, or Esc to keep the current ones." : "Press this anywhere to open the command box."}>
        <button
          className={recording ? "hotkey recording" : "hotkey"}
          onClick={() => setRecording(true)}
          onBlur={() => setRecording(false)}
          title="Click, then press new keys. Avoid Fn, which Wispr Flow uses."
          onKeyDown={(e) => {
            if (!recording) return;
            e.preventDefault();
            e.stopPropagation(); // Esc here cancels recording, not Settings.
            if (e.key === "Escape") return setRecording(false);
            const combo = comboFromEvent(e);
            if (combo) {
              setRecording(false);
              persist({ ...settings, hotkey: combo });
            }
          }}
        >
          {recording ? "Press new keys" : prettyHotkey(settings.hotkey)}
        </button>
      </Row>
      <Row icon="power" label="Open at login" htmlFor="login" description="Start Voxi in the menu bar when you log in.">
        <input
          id="login"
          className="switch"
          type="checkbox"
          checked={settings.startAtLogin}
          onChange={(e) => persist({ ...settings, startAtLogin: e.target.checked })}
        />
      </Row>
    </Group>
    <Group title="Dictation" foot={linkError ? <span className="warn" role="alert">{linkError}</span> : "Requires Apple silicon and macOS 15 or later. Other dictation apps work too."}>
      <Row icon="mic" label={<>Hex by Anomaly <span className="pill">Recommended</span></>} description="Free, open source dictation that runs on your Mac.">
        <button className="secondary small" onClick={() => { setLinkError(null); api.openHex().catch((e) => setLinkError(errorMessage(e))); }}>Get Hex</button>
      </Row>
    </Group>
    <Group title="Updates">
      <UpdatesRow />
    </Group>
    <Group title="Privacy">
      <ReportsRow />
    </Group>
  </>;
}

const mb = (bytes: number) => (bytes / 1_000_000).toFixed(1);

/** The installed version, and a button that fetches and installs the latest
 *  release, with a progress bar and its notes while it downloads. */
function UpdatesRow() {
  const [version, setVersion] = useState("");
  const [state, setState] = useState<UpdateState>({ k: "idle" });

  useEffect(() => {
    appVersion().then(setVersion);
  }, []);

  const busy = state.k === "checking" || state.k === "downloading" || state.k === "restarting";
  const status =
    state.k === "checking"
      ? "Checking…"
      : state.k === "downloading"
        ? state.total
          ? `Downloading ${state.version}: ${mb(state.done)} of ${mb(state.total)} MB`
          : `Downloading ${state.version}…`
        : state.k === "restarting"
          ? `Installed ${state.version}. Restarting…`
          : state.k === "error"
            ? state.message
            : null;
  const fraction = state.k === "downloading" && state.total ? Math.min(state.done / state.total, 1) : null;
  const notes = state.k === "downloading" || state.k === "restarting" ? state.notes : [];

  return (
    <Row icon="update" className="updates" label={<>Voxi {version} {state.k === "latest" && <span className="pill ok">Up to date</span>}</>}
      description={status ? <span className={state.k === "error" ? "warn" : undefined} role="status">{status}</span> : "Your settings carry over."}>
      <button className="secondary small" disabled={busy} onClick={() => checkAndInstall(setState)}>
        Check for updates
      </button>
      {(state.k === "downloading" || state.k === "restarting" || notes.length > 0) && (
        <div className="update-status">
          {(state.k === "downloading" || state.k === "restarting") && (
            <div
              className="progress"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={fraction === null ? undefined : Math.round(fraction * 100)}
              data-indeterminate={fraction === null && state.k === "downloading" ? "" : undefined}
            >
              <span style={{ width: `${state.k === "restarting" ? 100 : (fraction ?? 0.3) * 100}%` }} />
            </div>
          )}
          {notes.length > 0 && (
            <ul className="notes">
              {notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          )}
        </div>
      )}
    </Row>
  );
}

/** Wrong-result reports kept on this Mac, with ways to see or clear them. */
function ReportsRow() {
  const [reports, setReports] = useState<Reports | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    api.reportsSummary().then(setReports).catch(() => setReports(null));
  }, []);

  const reportAction = async (run: () => Promise<Reports | void>) => {
    try {
      const r = await run();
      if (r) setReports(r);
      setNote(null);
    } catch (e) {
      setNote(errorMessage(e));
    }
  };

  const count = reports?.count ?? 0;
  return (
    <Row icon="shield" label="Wrong-result reports"
      description={note ? <span className="warn" role="alert">{note}</span>
        : `${count === 0 ? "None" : count} saved. Nothing leaves this Mac.`}>
      <button className="secondary small" disabled={!count} onClick={() => reportAction(api.revealReports)}>Show in Finder</button>
      <button className="link remove" disabled={!count} onClick={() => reportAction(api.clearReports)}>Clear</button>
    </Row>
  );
}

/** Saved phrases that run several commands at once, with one preview and one
 *  undo: "run my usuals" -> move today's screenshots, clear installers. Each
 *  shortcut stays folded to its phrase until opened; Done saves it. */
function ShortcutsTab({ settings, persist }: TabProps) {
  const [list, setList] = useState<Shortcut[]>(settings.shortcuts);
  const [editing, setEditing] = useState<number | null>(null);
  const [showLengthError, setShowLengthError] = useState(false);
  const update = (i: number, s: Shortcut) => { setList((l) => l.map((x, j) => (j === i ? s : x))); setShowLengthError(false); };
  const count = (s: Shortcut) => s.commands.filter((c) => c.trim()).length;

  // Backend save_settings validates every shortcut: one oversized entry
  // refuses the whole batch, so no save path may proceed while one is invalid.
  const commit = async (next: Shortcut[]) => {
    if (next.some((s) => shortcutLengthIssue(s).any)) { setShowLengthError(true); return; }
    setShowLengthError(false);
    // Leave the editor / drop the row only once the save actually succeeded.
    if (await persist({ ...settings, shortcuts: next })) { setList(next); setEditing(null); }
  };
  const add = () => {
    setList((l) => [...l, { phrase: "", commands: [] }]);
    setEditing(list.length);
  };
  const foot = "When Voxi misses a phrase and you then run a command that works, it offers to learn the phrase. Learned phrases appear here too.";

  if (list.length === 0) {
    return <>
      <div className="empty">
        <span className="empty-icon"><Icon name="shortcuts" /></span>
        <p className="empty-title">No shortcuts yet</p>
        <p className="empty-desc">Say “run my usuals” to move today's screenshots and clear old installers, all in one preview.</p>
        <button className="secondary small" onClick={add}>Add shortcut</button>
      </div>
      <p className="group-foot">{foot}</p>
    </>;
  }

  return (
    <Group title="Saved shortcuts" action={editing === null && <button className="link strong" onClick={add}>+ Add</button>} foot={foot}>
      {list.map((s, i) =>
        editing === i ? (
          <ShortcutEditor
            key={i}
            shortcut={s}
            errorId={`shortcut-length-${i}`}
            canSave={!list.some((x) => shortcutLengthIssue(x).any)}
            canRemove={!list.filter((_, j) => j !== i).some((x) => shortcutLengthIssue(x).any)}
            onChange={(next) => update(i, next)}
            showLengthError={showLengthError}
            onDone={() => commit(list)}
            onRemove={() => commit(list.filter((_, j) => j !== i))}
          />
        ) : (
          <li className="row clickable" key={i}>
            <button className="shortcut-row" onClick={() => {
              // An over-limit row stays open so its error remains visible;
              // collapsing it would hide why the next row's Done is disabled.
              if (editing === null || !list[editing] || !shortcutLengthIssue(list[editing]).any) setEditing(i);
            }}>
              <span className="row-icon"><Icon name="shortcuts" /></span>
              <span className="row-copy">
                <span className="row-label">“{s.phrase || "Untitled"}”</span>
                <span className="row-desc">{s.commands.filter((c) => c.trim()).slice(0, 2).join(" · ") || "No commands yet"}</span>
              </span>
              <span className="pill">{count(s)} {count(s) === 1 ? "command" : "commands"}</span>
              <span className="chevron" aria-hidden="true">›</span>
            </button>
          </li>
        ),
      )}
    </Group>
  );
}

/** One expanded shortcut row. The message is the backend's exact refusal,
 *  linked to whichever field is over the bound; text is never truncated. */
function ShortcutEditor({
  shortcut, errorId, canSave, canRemove, showLengthError, onChange, onDone, onRemove,
}: {
  shortcut: Shortcut;
  errorId: string;
  canSave: boolean;
  canRemove: boolean;
  showLengthError: boolean;
  onChange: (s: Shortcut) => void;
  onDone: () => void;
  onRemove: () => void;
}) {
  const issue = shortcutLengthIssue(shortcut);
  return (
    <li className="row shortcut">
      <div className="shortcut-head">
        <input
          className="phrase"
          value={shortcut.phrase}
          placeholder="run my usuals"
          aria-label="Phrase to say"
          aria-invalid={issue.phrase || undefined}
          aria-describedby={issue.phrase ? errorId : undefined}
          autoFocus={!shortcut.phrase}
          onChange={(e) => onChange({ ...shortcut, phrase: e.target.value })}
        />
        <button className="link strong" disabled={!canSave} onClick={onDone}>
          Done
        </button>
      </div>
      <textarea
        writingsuggestions="false"
        spellCheck={false}
        autoCorrect="off"
        className="plain commands"
        rows={Math.max(2, shortcut.commands.length)}
        placeholder={"move screenshots from today to Screenshots\nclean up Downloads older than 90 days"}
        aria-label="Commands, one per line"
        aria-invalid={issue.commands || undefined}
        aria-describedby={issue.commands ? errorId : undefined}
        value={shortcut.commands.join("\n")}
        onChange={(e) => onChange({ ...shortcut, commands: e.target.value.split("\n") })}
      />
      <p className="shortcut-error hint warn" id={errorId} role="alert">{issue.any || showLengthError ? TOO_LONG : ""}</p>
      <button className="link remove" disabled={!canRemove} onClick={onRemove}>
        Remove shortcut
      </button>
    </li>
  );
}

/** "~/Work" from "/Users/me/Work/", the form allowed_folders returns. */
const shownFolder = (path: string) => path.replace(/^\/Users\/[^/]+/, "~").replace(/\/$/, "");

/** The four standard folders are always allowed; more can be added. Below
 *  them, the nicknames Voxi knows folders by. */
function FoldersTab({ settings, persist, setBusy }: TabProps & { setBusy: (b: boolean) => void }) {
  const [folders, setFolders] = useState<string[]>([]);
  const [adding, setAdding] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const extra = settings.extraFolders.filter((f) => f.trim());

  useEffect(() => {
    api.allowedFolders().then(setFolders);
  }, [settings]);

  const add = async (path: string) => {
    if (!path) return;
    const shown = shownFolder(path);
    if (folders.includes(shown) || extra.some((x) => shownFolder(x) === shown)) {
      setAdding("");
      setNote(`“${shown}” is already allowed.`);
      return;
    }
    if (await persist({ ...settings, extraFolders: [...extra, path] })) {
      const now = await api.allowedFolders();
      setFolders(now);
      setAdding("");
      setNote(now.includes(shown) ? null : `Couldn't find “${path}”. Check the path and try again.`);
    }
  };

  // The picker takes focus, which would otherwise close the window.
  const choose = async () => {
    setBusy(true);
    try {
      const path = await api.pickFolder();
      if (path) await add(path);
    } catch (e) {
      setNote(`Couldn't open the folder picker: ${errorMessage(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const builtIn = folders.filter((f) => !extra.some((x) => shownFolder(x) === f));

  return <>
    <Group title="Allowed folders" foot={note ? <span className="warn" role="status">{note}</span> : "Voxi only changes files inside these folders."}>
      <Row className="add-row">
        <input
          className="path"
          value={adding}
          placeholder="Add a folder, like /Volumes/External"
          aria-label="Folder path to add"
          onChange={(e) => setAdding(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              add(adding.trim());
            }
          }}
        />
        {adding.trim()
          ? <button className="secondary small" onClick={() => add(adding.trim())}>Add</button>
          : <button className="secondary small" onClick={choose}>Choose…</button>}
      </Row>
      {builtIn.map((f) => (
        <Row key={f} className="compact" icon="folders" label={<span className="pref-folder">{f}</span>}>
          <span className="pill">Built-in</span>
        </Row>
      ))}
      {extra.map((f) => (
        <Row key={f} className="compact" icon="folders" label={<span className="pref-folder">{tildify(f)}</span>}>
          <span className="pill ok">Added</span>
          <button className="link remove" onClick={() => persist({ ...settings, extraFolders: extra.filter((x) => x !== f) })}>
            Remove
          </button>
        </Row>
      ))}
    </Group>
    <NicknamesGroup settings={settings} persist={persist} setBusy={setBusy} />
  </>;
}

/** Your names for folders, so “move it to dev” finds the right one. */
function NicknamesGroup({ settings, persist, setBusy }: TabProps & { setBusy: (b: boolean) => void }) {
  const [name, setName] = useState("");
  const [note, setNote] = useState<string | null>(null);

  // The picker takes focus, which would otherwise close the window.
  const choose = async () => {
    const said = name.trim();
    if (!said) return setNote("Type the name you'll say first, like “dev”.");
    setBusy(true);
    try {
      const path = await api.pickFolder();
      if (path && await persist({ ...settings, nicknames: [...settings.nicknames, { name: said, path }] })) {
        setName("");
        setNote(null);
      }
    } catch (e) {
      setNote(`Couldn't open the folder picker: ${errorMessage(e)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Group title="Nicknames" foot={note ? <span className="warn" role="status">{note}</span> : "Say “move it to dev” and Voxi knows where. It also offers to remember a folder after you pick one."}>
      <Row className="add-row">
        <input
          className="path"
          value={name}
          placeholder="Name to say, like dev"
          aria-label="Nickname to say"
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") { e.preventDefault(); choose(); }
          }}
        />
        <button className="secondary small" onClick={choose}>Choose folder…</button>
      </Row>
      {settings.nicknames.map((n) => (
        <Row key={n.name} className="compact" icon="tag" label={`“${n.name}”`} description={tildify(n.path)}>
          <button className="link remove" onClick={() => persist({ ...settings, nicknames: settings.nicknames.filter((x) => x.name !== n.name) })}>
            Remove
          </button>
        </Row>
      ))}
    </Group>
  );
}

// ---- Folder access ---------------------------------------------------------------

/** Asks macOS for every folder up front, so commands never stop to ask. On
 *  first launch this is the welcome page. */
