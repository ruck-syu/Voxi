import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { api, errorMessage, inTauri, isConfirmError, tildify, type CommandRequestResult, type Outcome, type PreviewRow, type ResultScope, type Review, type RunResult, type SearchStats, type Settings } from "./api";
import { readChoice } from "./choice";
import { version as appVersion } from "../package.json";
import examples from "./examples.json";
import { AccessView } from "./access";
import { SettingsView } from "./settings";
import { maxCommand, maxNote, reportFields, sanitizeWrongReport, type ReportedActions, type ReportOutcome, type WrongReportFieldErrors } from "./wrong-report";
import { MAX_COMMAND_BYTES, TOO_LONG, row, utf8Bytes } from "./ui";

type View = "command" | "settings" | "welcome";

/** Request ids for planning, unique across command sessions: a remounted
 *  view must never reuse an id an older, cancelled request still holds. */
let lastRequest = 0;
type Phase =
  | { k: "ready" }
  | { k: "parsing" }
  | { k: "outcome"; outcome: Outcome; refused?: string; searchStats?: SearchStats }
  | { k: "running"; title: string; canStop: boolean; stopping: boolean; stopError?: string }
  | { k: "done"; result: RunResult; actionCounts: ReportedActions; wasUndo: boolean; searchStats?: SearchStats }
  | { k: "error"; message: string };

/** Waiting for the answer to "which one?": the command being planned and
 *  the picks made so far (one command can need several). */
type Asking = { command: string; choices: Record<string, string>; name: string; candidates: string[]; folders: boolean; replaces?: number };

/** Something Voxi can learn from what just happened, offered once. */
type Offer =
  | { k: "folder"; name: string; path: string; saved?: string }
  | { k: "phrase"; phrase: string; command: string; saved?: string };

/** How long a missed phrase stays teachable by the command that follows it. */
const TEACH_WINDOW_MS = 3 * 60_000;


/** Example commands, one per kind of feature. A Rust test checks each parses. */
const EXAMPLES: readonly string[] = examples;

/** A few examples to click, picked once when shown. */
function ExampleList({ onFill }: { onFill: (text: string) => void }) {
  const [picked] = useState(() => someExamples(4));
  return (
    <ul className="examples">
      {picked.map((ex, i) => (
        <li key={ex} style={row(i)}>
          <button className="example" onClick={() => onFill(ex)}>
            {ex}
          </button>
        </li>
      ))}
    </ul>
  );
}

/** `count` examples starting at a random one, so each opening shows different ones. */
function someExamples(count: number): string[] {
  const start = Math.floor(Math.random() * EXAMPLES.length);
  return Array.from({ length: Math.min(count, EXAMPLES.length) }, (_, i) => EXAMPLES[(start + i) % EXAMPLES.length]);
}


const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const CLOSE_MS = 140;

export default function App() {
  const [view, setView] = useState<View>("command");
  const [review, setReview] = useState<Review | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  // Settings that failed to load must not hold the window back forever.
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [open, setOpen] = useState(!inTauri);
  const [pendingShow, setPendingShow] = useState<number | null>(null);
  const [showRequestId, setShowRequestId] = useState(0);
  const [setupNotice, setSetupNotice] = useState<string | null>(null);
  const [commandSession, setCommandSession] = useState(0);
  const [fontsReady, setFontsReady] = useState(false);
  const [height, setHeight] = useState<number | null>(null);
  const [glide, setGlide] = useState(false);
  const innerRef = useRef<HTMLDivElement>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const blurTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const showId = useRef(0);
  const awaitingShow = useRef<number | null>(null);

  const cancelBlurClose = useCallback(() => {
    if (blurTimer.current !== null) clearTimeout(blurTimer.current);
    blurTimer.current = null;
  }, []);
  const cancelClose = useCallback(() => {
    cancelBlurClose();
    if (closeTimer.current !== null) clearTimeout(closeTimer.current);
    closeTimer.current = null;
  }, [cancelBlurClose]);
  useEffect(() => cancelClose, [cancelClose]);

  // Plays the close animation, then hides the window. The card stays in its
  // closed state while hidden, so the next show animates in from there.
  const close = useCallback(() => {
    cancelClose();
    const requestId = showId.current;
    awaitingShow.current = null;
    setOpen(false);
    setPendingShow(null);
    if (inTauri) void invoke("dismiss_window", { requestId, animate: !reducedMotion() });
    closeTimer.current = setTimeout(() => {
      closeTimer.current = null;
      void api.hide(requestId);
    }, reducedMotion() ? 0 : CLOSE_MS);
  }, [cancelClose]);

  // Clicking outside the window (another app, the desktop, the menu bar)
  // closes it, like Spotlight. Not while a command is being planned or run:
  // a macOS folder-access prompt can take focus mid-command. And not in the
  // moment right after opening, while macOS is still handing over focus.
  const busyRef = useRef(false);
  const executingRef = useRef(false);
  const setBusy = useCallback((busy: boolean, executing = false) => {
    busyRef.current = busy;
    executingRef.current = executing;
  }, []);
  const auxiliaryOperations = useRef(new Set<symbol>());
  const beginOperation = useCallback(() => {
    const owner = Symbol();
    auxiliaryOperations.current.add(owner);
    return () => { auxiliaryOperations.current.delete(owner); };
  }, []);
  const isBusy = useCallback(() => busyRef.current || auxiliaryOperations.current.size > 0, []);
  const shownAt = useRef(0);
  useEffect(() => {
    if (!inTauri) return;
    const onBlur = () => {
      if (isBusy()) return;
      cancelBlurClose();
      const requestId = showId.current;
      // WebKit can briefly blur during activation, paste, or text services.
      // A blur inside the opening grace still needs a deferred check: wait
      // for the grace to end, then for focus to settle, before closing.
      const graceLeft = Math.max(0, 300 - (performance.now() - shownAt.current));
      blurTimer.current = setTimeout(() => {
        blurTimer.current = null;
        if (document.hasFocus() || isBusy() || requestId !== showId.current
          || awaitingShow.current !== null || performance.now() - shownAt.current < 300) return;
        close();
      }, graceLeft + 150);
    };
    window.addEventListener("blur", onBlur);
    window.addEventListener("focus", cancelBlurClose, true);
    return () => {
      cancelBlurClose();
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("focus", cancelBlurClose, true);
    };
  }, [close, cancelBlurClose, isBusy]);

  useEffect(() => {
    api.getSettings().then((s) => {
      setSettings(s);
      if (!s.onboarded) setView("welcome");
    }).catch((e) => setSettingsError(errorMessage(e)));
    if (!inTauri) return;
    let disposed = false;
    const unShown = listen<{ requestId: number; mode: View }>("voxi://shown", (e) => {
      const { mode, requestId } = e.payload;
      cancelClose();
      showId.current = requestId;
      setShowRequestId(requestId);
      setSetupNotice(null);
      if (mode === "welcome") {
        void api.openingContext(requestId).then((opening) => {
          if (!disposed && showId.current === requestId) setSetupNotice(opening?.notice ?? null);
        }).catch(() => {});
      }
      awaitingShow.current = requestId;
      // Keep the confirmation and Stop control if the window is reopened mid-run,
      // and don't unmount Settings/Access while a folder picker or save is pending.
      if (!executingRef.current && auxiliaryOperations.current.size === 0) {
        setCommandSession(requestId);
        setView((v) => (mode === "command" && v === "welcome" ? v : mode));
      }
      shownAt.current = performance.now();
      setOpen(false);
      setPendingShow(requestId);
    });
    const unPresented = listen<number>("voxi://presented", ({ payload: requestId }) => {
      if (requestId !== showId.current || awaitingShow.current !== requestId) return;
      awaitingShow.current = null;
      setPendingShow(null);
      setOpen(true);
      shownAt.current = performance.now();
      window.dispatchEvent(new Event("voxi-focus"));
    });
    const unHide = listen("voxi://hide", close);
    const unOutsideClick = listen<number>("voxi://outside-click", ({ payload: requestId }) => {
      if (disposed || requestId !== showId.current || awaitingShow.current !== null || isBusy()) return;
      // A real click in another app is not the transient focus handoff that
      // blur's opening grace period protects against.
      close();
    });
    Promise.all([unShown, unPresented, unHide, unOutsideClick]).then(() => { if (!disposed) void invoke("frontend_ready"); });
    return () => {
      disposed = true;
      unShown.then((f) => f());
      unPresented.then((f) => f());
      unHide.then((f) => f());
      unOutsideClick.then((f) => f());
    };
  }, [close, cancelClose, isBusy]);

  useEffect(() => {
    let disposed = false;
    document.fonts.ready.then(() => { if (!disposed) setFontsReady(true); });
    return () => { disposed = true; };
  }, []);

  // Layout is measured after settings and fonts settle, while the native window
  // is still hidden. The acknowledgement resizes and reveals it in one operation.
  useLayoutEffect(() => {
    if (!inTauri || pendingShow === null || (!settings && !settingsError) || !fontsReady || !innerRef.current) return;
    const requestId = pendingShow;
    const height = Math.ceil(innerRef.current.offsetHeight);
    void invoke("present_window", { requestId, height, animate: !reducedMotion() });
  }, [pendingShow, settings, settingsError, fontsReady, view]);

  // The window is the frosted card, so it is sized to the content and macOS
  // animates the resize. In a browser preview the card glides instead.
  useLayoutEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      const h = Math.ceil(el.offsetHeight);
      if (inTauri) invoke("fit_window", { height: h, animate: !reducedMotion() });
      else setHeight(h);
    });
    ro.observe(el);
    // Only glide after the first measurement, never from zero.
    requestAnimationFrame(() => requestAnimationFrame(() => setGlide(true)));
    return () => ro.disconnect();
  }, []);

  return (
    <div className="card" data-open={open || undefined} data-glide={glide || undefined} style={height ? { height } : undefined}>
      <div className="card-inner" ref={innerRef}>
        {view === "command" && <CommandView
            key={commandSession}
            showRequestId={showRequestId}
            latestShowId={showId}
            open={open}
            onClose={close}
            onBusy={setBusy}
            beginOperation={beginOperation}
            onSettings={() => setView("settings")}
            onSettingsChanged={setSettings}
            showExamples={settings ? !settings.ranFirstCommand : false}
            onFirstCommand={() => setSettings((s) => (s && !s.ranFirstCommand ? { ...s, ranFirstCommand: true } : s))}
            review={review}
            onReviewStarted={() => setReview(null)}
          />}
        {view !== "command" && !settings && settingsError && (
          <section className="body">
            <p className="title warn" role="alert">Voxi couldn't load its settings. {settingsError}</p>
          </section>
        )}
        {view === "settings" && settings && (
          <SettingsView
            settings={settings}
            onSaved={setSettings}
            onClose={() => setView("command")}
            onReview={(task) => {
              setReview(task);
              setView("command");
            }}
            beginOperation={beginOperation}
          />
        )}
        {view === "welcome" && settings && (
          <AccessView
            settings={settings}
            firstRun
            notice={setupNotice}
            beginOperation={beginOperation}
            onDone={async () => {
              const s = await api.saveSettings({ ...settings, onboarded: true });
              setSettings(s);
              setView("command");
            }}
          />
        )}
      </div>
    </div>
  );
}

/** Cycles example commands through the placeholder, typed out like a
 *  person would, while the box is empty. Shown only until the first command
 *  runs; inactive, it is the fixed "Type or dictate a command". */
function useTypedPlaceholder(active: boolean): string {
  const base = "Type or dictate a command";
  const [shown, setShown] = useState(base);
  useEffect(() => {
    if (!active || reducedMotion()) {
      setShown(base);
      return;
    }
    let cancelled = false;
    let timer: number | undefined;
    const wait = (ms: number) => new Promise<void>((r) => (timer = window.setTimeout(r, ms)));
    (async () => {
      setShown(base);
      await wait(2600);
      for (let i = Math.floor(Math.random() * EXAMPLES.length); !cancelled; i = (i + 1) % EXAMPLES.length) {
        const ex = EXAMPLES[i];
        for (let n = 1; n <= ex.length && !cancelled; n++) {
          setShown(ex.slice(0, n));
          await wait(38 + Math.random() * 40);
        }
        await wait(2200);
        for (let n = ex.length; n >= 0 && !cancelled; n--) {
          setShown(ex.slice(0, n));
          await wait(14);
        }
        await wait(350);
      }
    })();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [active]);
  return shown;
}

// ---- Command -----------------------------------------------------------------

function CommandView({
  showRequestId,
  latestShowId,
  open,
  onClose,
  onBusy,
  beginOperation,
  onSettings,
  onSettingsChanged,
  showExamples,
  onFirstCommand,
  review,
  onReviewStarted,
}: {
  showRequestId: number;
  latestShowId: RefObject<number>;
  open: boolean;
  onClose: () => void;
  onBusy: (busy: boolean, executing?: boolean) => void;
  /** Keeps the window open (and mounted) while a side operation runs. */
  beginOperation: () => () => void;
  onSettings: () => void;
  onSettingsChanged: (s: Settings) => void;
  /** Cycle example commands in the empty box: only until the first command runs. */
  showExamples: boolean;
  /** The backend records the first run; this keeps the open window in step. */
  onFirstCommand: () => void;
  /** A scheduled task to preview again, planned as soon as this view opens. */
  review: Review | null;
  onReviewStarted: () => void;
}) {
  const [text, setText] = useState("");
  const [phase, setPhase] = useState<Phase>({ k: "ready" });
  const [reviewingReport, setReviewingReport] = useState(false);
  const [asking, setAsking] = useState<Asking | null>(null);
  // The last command, echoed above its result once the box is cleared.
  const [said, setSaid] = useState<string | null>(null);
  const [heard, setHeard] = useState<string | null>(null);
  const [resultScope, setResultScope] = useState<ResultScope | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const ruleRef = useRef<HTMLSpanElement>(null);
  const planShownAt = useRef(0);
  // Files sent from Finder arrive as the new window request's results list.
  const [selectionNotice, setSelectionNotice] = useState<string | null>(null);
  // The results list this view knows, so closing it clears only that list
  // and never one that arrived after (a new Finder selection).
  const scopeGeneration = useRef<number | null>(null);
  useEffect(() => {
    // The native webview starts hidden. Wait for the request that opens it.
    if (inTauri && showRequestId === 0) return;
    let live = true;
    void api.openingContext(showRequestId).then((o) => {
      // A command or a newer show request may already have replaced this list.
      if (!live || !o || (scopeGeneration.current !== null && o.generation < scopeGeneration.current)) return;
      scopeGeneration.current = o.generation;
      setResultScope(o.scope);
      setSelectionNotice(o.notice);
    }).catch(() => {});
    return () => { live = false; };
  }, [showRequestId]);
  const fromFinder = resultScope?.label === FINDER_LABEL;
  const activeRun = useRef<{ id: number; canStop: boolean; stopping: boolean } | null>(null);
  const requestId = useRef(0);
  const [searched, setSearched] = useState(0);
  // The command behind the result on screen, exactly as entered.
  const lastCommand = useRef("");
  // The last phrase Voxi didn't understand, which the next run can teach.
  const missed = useRef<{ text: string; at: number } | null>(null);
  const [offer, setOffer] = useState<Offer | null>(null);
  // Steps the user unticked, for the plan with this id only.
  const [skip, setSkip] = useState<{ id: number; steps: number[] }>({ id: -1, steps: [] });
  const skippedFor = (id: number) => skip.id === id ? skip.steps : [];
  const toggleSkip = useCallback((id: number, step: number) => setSkip((prev) => {
    const steps = prev.id === id ? prev.steps : [];
    return { id, steps: steps.includes(step) ? steps.filter((s) => s !== step) : [...steps, step] };
  }), []);
  // A folder picked for an ambiguous name, offered as a nickname after the plan.
  const pickedFolder = useRef<{ name: string; path: string } | null>(null);
  // Same bound the backend applies to every command; checked before trimming.
  const tooLong = utf8Bytes(text) > MAX_COMMAND_BYTES;

  useEffect(() => () => {
    (requestId.current = ++lastRequest);
    api.cancel();
    if (scopeGeneration.current !== null) void api.clearResultScope(scopeGeneration.current);
  }, []);

  useEffect(() => {
    if (phase.k !== "parsing") return;
    const request = requestId.current;
    const timer = window.setInterval(() => {
      api.searchProgress(request).then((count) => {
        if (request === requestId.current) setSearched(count);
      }).catch(() => {});
    }, 150);
    return () => window.clearInterval(timer);
  }, [phase]);

  const focusInput = useCallback(() => {
    // Focus after the window has become key, or macOS drops it.
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el || el.closest("[hidden]")) return;
      el.focus({ preventScroll: true });
      el.setSelectionRange(el.value.length, el.value.length);
    });
  }, []);

  // Each native show request mounts a fresh command session before layout is
  // measured. Focus events only focus the input; they do not reset a live session.
  useEffect(() => {
    if (reviewingReport) return;
    window.addEventListener("voxi-focus", focusInput);
    window.addEventListener("focus", focusInput);
    focusInput();
    return () => {
      window.removeEventListener("voxi-focus", focusInput);
      window.removeEventListener("focus", focusInput);
    };
  }, [focusInput, reviewingReport]);

  // Grow the text box with multi-line commands.
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    const fit = () => {
      el.style.height = "0px";
      el.style.height = `${el.scrollHeight}px`;
    };
    fit();
    document.fonts.ready.then(fit);
  }, [text]);

  const submit = useCallback(
    async (command: string, picked: Record<string, string> = {}, undoEntryId?: number, replaces?: number) => {
      if (!command.trim()) return;
      // Never send an oversized command, and never truncate one to fit: the
      // text stays in the box with an inline message until it is shortened.
      if (utf8Bytes(command) > MAX_COMMAND_BYTES) return;
      // Clear the box straight away so the next command starts fresh; the
      // command stays visible as the echo above its result.
      setSaid(command.trim().replace(/[.!?]+$/, ""));
      setText("");
      lastCommand.current = command.trim();
      setOffer(null);
      const folderPick = pickedFolder.current;
      pickedFolder.current = null;
      const request = (requestId.current = ++lastRequest);
      setSearched(0);
      setPhase({ k: "parsing" });
      setHeard(null);
      try {
        const { outcome, scope, generation, search_stats: searchStats } = await api.plan(command, picked, request, undoEntryId, replaces);
        if (request !== requestId.current) return;
        if (generation !== null && (scopeGeneration.current === null || generation >= scopeGeneration.current)) {
          scopeGeneration.current = generation;
          setResultScope(scope);
        }
        setPhase({ k: "outcome", outcome, searchStats });
        if (outcome.status === "plan") planShownAt.current = performance.now();
        if (outcome.status === "not_understood") missed.current = { text: command.trim(), at: Date.now() };
        // Once the picked folder led somewhere, offer to remember it by name.
        if (folderPick && (outcome.status === "plan" || outcome.status === "info")) setOffer({ k: "folder", ...folderPick });
        if (outcome.status === "ambiguous") {
          // The answer will be dictated too.
          setAsking({ command, choices: picked, name: outcome.name, candidates: outcome.candidates, folders: outcome.folders, replaces });
        } else {
          setAsking(null);
        }
        // Focus stays in the box for every outcome, previews included, so
        // the next thing dictated always lands there. Enter runs a preview.
        focusInput();
      } catch (e) {
        if (request !== requestId.current) return;
        // The choice list is gone, so Enter and spoken numbers must stop picking from it.
        setAsking(null);
        setPhase({ k: "error", message: errorMessage(e) });
      }
    },
    [focusInput],
  );

  const pick = useCallback(
    (index: number) => {
      if (!asking) return;
      const path = asking.candidates[index];
      if (!path) return;
      if (asking.folders) pickedFolder.current = { name: asking.name, path };
      submit(asking.command, { ...asking.choices, [asking.name]: path }, undefined, asking.replaces);
    },
    [asking, submit],
  );

  // Reviewing a scheduled task previews its command as if it were typed
  // again; confirming that preview replaces the task.
  useEffect(() => {
    if (!review) return;
    onReviewStarted();
    void submit(review.command, review.choices, undefined, review.id);
  }, [review, onReviewStarted, submit]);

  const stopAsking = useCallback(() => {
    setAsking(null);
  }, []);

  const confirm = useCallback(async () => {
    if (activeRun.current || phase.k !== "outcome" || phase.outcome.status !== "plan") return;
    const { outcome: plan, searchStats } = phase;
    const skipped = skip.id === plan.id ? skip.steps : [];
    if (skipped.length >= plan.total) return;
    const canStop = plan.firstRun === "immediate" || plan.firstRun === "runNowAndRepeat";
    activeRun.current = { id: plan.id, canStop, stopping: false };
    // An offer already answered is done; one still open stays for after the run.
    setOffer((current) => (current?.saved ? null : current));
    setPhase({ k: "running", title: plan.title, canStop, stopping: false });
    try {
      const result = await api.confirm(plan.id, skipped);
      const after = await api.openingContext(latestShowId.current).catch(() => null);
      if (after && (scopeGeneration.current === null || after.generation >= scopeGeneration.current)) {
        scopeGeneration.current = after.generation;
        setResultScope(after.scope);
        setSelectionNotice(after.notice);
      }
      setPhase({ k: "done", result, actionCounts: plan.actionCounts, wasUndo: plan.isUndo, searchStats });
      onFirstCommand();
      // A phrase Voxi missed, followed soon by a command that worked, can
      // be taught: next time the phrase runs this command.
      const miss = missed.current;
      missed.current = null;
      // Only a run that changed files teaches: scheduling one changes nothing yet.
      if (!result.error && !result.cancelled && !plan.isUndo && result.done > 0 && miss && Date.now() - miss.at < TEACH_WINDOW_MS && miss.text !== lastCommand.current) {
        setOffer({ k: "phrase", phrase: miss.text, command: lastCommand.current });
      }
      focusInput();
    } catch (e) {
      // A refusal before anything ran (a file changed, a step can't be
      // left out) keeps the preview, so it can be fixed or cancelled.
      if (isConfirmError(e) && e.previewKept) setPhase({ k: "outcome", outcome: plan, refused: e.message, searchStats });
      else setPhase({ k: "error", message: errorMessage(e) });
    } finally {
      activeRun.current = null;
    }
  }, [phase, focusInput, skip, latestShowId, onFirstCommand]);

  const stop = useCallback(async () => {
    const run = activeRun.current;
    if (!run?.canStop || run.stopping) return;
    run.stopping = true;
    setPhase((current) => current.k === "running" ? { ...current, stopping: true, stopError: undefined } : current);
    try {
      // False: the run already finished; its result arrives on its own.
      if (!(await api.stopExecution(run.id)) && activeRun.current === run) {
        run.stopping = false;
        setPhase((current) => current.k === "running" ? { ...current, stopping: false } : current);
      }
    } catch (e) {
      if (activeRun.current !== run) return;
      run.stopping = false;
      setPhase((current) => current.k === "running" ? { ...current, stopping: false, stopError: errorMessage(e) } : current);
    }
  }, []);

  const reset = useCallback(() => {
    const request = (requestId.current = ++lastRequest);
    setSelectionNotice(null);
    const known = scopeGeneration.current;
    void Promise.all([api.cancel(), known === null ? undefined : api.clearResultScope(known)]).catch((e) => {
      if (request === requestId.current) setPhase({ k: "error", message: errorMessage(e) });
    });
    stopAsking();
    setText("");
    setResultScope(null);
    setSearched(0);
    setHeard(null);
    setSaid(null);
    setOffer(null);
    lastCommand.current = "";
    missed.current = null;
    pickedFolder.current = null;
    setPhase({ k: "ready" });
    focusInput();
  }, [focusInput, stopAsking]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing || reviewingReport) return;
      if (e.key === "Escape") {
        e.preventDefault();
        if (e.repeat) return;
        if (activeRun.current) { void stop(); return; }
        if (phase.k !== "ready" || text !== "" || resultScope) reset();
        else onClose();
      } else if (e.key === "," && e.metaKey) {
        e.preventDefault();
        if (!activeRun.current) onSettings();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase, text, resultScope, reset, stop, onSettings, onClose, reviewingReport]);

  const answer = (said: string) => {
    if (!asking) return;
    const choice = readChoice(said, asking.candidates);
    if (choice && "index" in choice) return pick(choice.index);
    if (choice && "cancel" in choice) return reset();
    // Not an answer: say so, and keep listening. A full new command (one
    // the parser would act on) needs Esc first, so a misheard number never
    // runs as a command.
    setHeard(said.trim().replace(/[.!?]+$/, ""));
    setText("");
    focusInput();
  };

  const onInputKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Up in an empty box brings back the last command to edit.
    if (e.key === "ArrowUp" && text === "" && said && !asking) {
      e.preventDefault();
      setText(said);
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (tooLong) return; // submit is disabled until the command fits
      if (activeRun.current) return;
      if (asking) return answer(text);
      const previewing = phase.k === "outcome" && phase.outcome.status === "plan";
      if (previewing && text.trim() === "") {
        // Enter on an empty box runs the preview. A held key, or an Enter
        // landing right as the preview appears, doesn't count: it was meant
        // for the command, not as a yes.
        if (e.repeat || performance.now() - planShownAt.current < 400) return;
        return confirm();
      }
      submit(text);
    }
  };

  // A keystroke presses the red rule; a dictated burst sweeps ink along it
  // and fades the new words in.
  const inkFeedback = (added: number) => {
    if (reducedMotion() || added <= 0) return;
    const rule = ruleRef.current;
    if (added > 3) {
      rule?.animate(
        [
          { backgroundPosition: "100% 0", transform: "scaleY(2)" },
          { backgroundPosition: "0% 0", transform: "scaleY(1)" },
        ],
        { duration: 520, easing: "cubic-bezier(.2,.8,.2,1)" },
      );
      inputRef.current?.animate([{ opacity: 0.25 }, { opacity: 1 }], { duration: 280, easing: "ease-out" });
    } else {
      rule?.animate([{ transform: "scaleY(1.9)" }, { transform: "scaleY(1)" }], { duration: 170, easing: "ease-out" });
    }
  };

  const onChange = (v: string) => {
    inkFeedback(v.length - text.length);
    setText(v);
    if (!matchMedia("(prefers-reduced-motion: reduce)").matches) api.trayTyping();
    // While asking, the box holds the answer; the question stays up.
    if (asking) return;
    // A pending plan is dropped as soon as a new command starts, so Enter can
    // never run a preview the box no longer matches. Other results (a list,
    // a "Done") stay visible while the next command is dictated.
    if (phase.k === "outcome" && phase.outcome.status === "plan") {
      api.cancel();
      setSaid(null);
      setPhase({ k: "ready" });
    }
  };

  const fill = (example: string) => {
    setAsking(null);
    setText(example);
    setPhase({ k: "ready" });
    focusInput();
  };

  const undo = () => {
    if (phase.k === "done" && phase.result.entryId !== null) {
      setAsking(null);
      submit("undo last", {}, phase.result.entryId);
    }
  };

  const executing = phase.k === "running";
  const busy = phase.k === "parsing" || executing;
  useLayoutEffect(() => {
    onBusy(busy, executing);
    return () => onBusy(false);
  }, [busy, executing, onBusy]);
  const placeholder = useTypedPlaceholder(showExamples && open && phase.k === "ready" && text === "");

  return (
    <>
      <div hidden={reviewingReport}>
        <div className="head" data-busy={busy || undefined}>
          <span className="rule" ref={ruleRef} aria-hidden="true" />
          <textarea
            ref={inputRef}
            className="command"
            rows={1}
            value={text}
            writingsuggestions="false"
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            placeholder={asking ? "Say or type a number" : placeholder}
            aria-label={asking ? "Which one?" : "Command"}
            aria-invalid={tooLong || undefined}
            aria-describedby="command-length-error"
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={onInputKey}
          />
          <p className="command-error hint warn" id="command-length-error" role="alert">{tooLong ? TOO_LONG : ""}</p>
        </div>
        {said && phase.k !== "ready" && (
          <p className="said" key={said}>
            “{said}”
          </p>
        )}
        {resultScope && phase.k !== "parsing" && <div className="result-scope" role="status">
          <span>{resultScope.count} {fromFinder ? (resultScope.count === 1 ? "file" : "files") : "results"} from {resultScope.label}{!fromFinder && resultScope.visible < resultScope.count ? ` · ${resultScope.visible} displayed` : ""}</span>
          <button className="link" disabled={phase.k === "running"} onClick={reset}>Clear scope</button>
        </div>}
        {phase.k === "ready" && (fromFinder || selectionNotice) && (
          <p className="quiet selection-hint" role="status">
            {selectionNotice && <span className="warn">{selectionNotice} </span>}
            {fromFinder && "Say what to do with them, such as “move these to Receipts”."}
          </p>
        )}
        <Body
          searched={searched}
          key={`body-${phaseKey(phase)}`}
          phase={phase}
          asking={asking}
          heard={heard}
          onConfirm={confirm}
          onCancel={reset}
          onStop={stop}
          onPick={pick}
          onFill={fill}
          onUndo={undo}
          skipped={phase.k === "outcome" && phase.outcome.status === "plan" ? skippedFor(phase.outcome.id) : []}
          onToggleSkip={toggleSkip}
        />
        {!busy && offer && <OfferRow key={offer.k === "folder" ? offer.path : offer.phrase} offer={offer} onSaved={(settings, text) => { onSettingsChanged(settings); setOffer((current) => (current === offer ? { ...offer, saved: text } : current)); }} onDone={() => { setOffer(null); focusInput(); }} />}
        {phase.k === "outcome" && phase.outcome.status === "not_understood" && (
          <CommandRequestRow key={requestId.current} original={lastCommand.current} onCancel={focusInput} beginOperation={beginOperation} />
        )}
      </div>
      {reportable(phase) && (
        <ReportRow key={`report-${requestId.current}-${phaseKey(phase)}`} command={lastCommand.current} phase={phase}
          onDone={focusInput} reviewing={reviewingReport} onReviewing={setReviewingReport} beginOperation={beginOperation} />
      )}
      {!reviewingReport && <Footer phase={phase} asking={!!asking} onSettings={onSettings} />}
    </>
  );
}

/** One explicit submission, with the window held open until it settles. */
function useFeedbackSend(beginOperation: () => () => void, failedMessage: string) {
  const [mode, setMode] = useState<"editing" | "sending" | "sent">("editing");
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const sending = mode === "sending";

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  useEffect(() => {
    if (!sending) return;
    const swallow = (event: KeyboardEvent) => {
      if (event.key === "Escape" || (event.key === "," && event.metaKey)) event.preventDefault();
    };
    window.addEventListener("keydown", swallow, true);
    return () => window.removeEventListener("keydown", swallow, true);
  }, [sending]);

  const submit = async (post: () => Promise<CommandRequestResult>, onSent: () => void, onError?: () => void) => {
    if (mode !== "editing" || inFlight.current) return;
    inFlight.current = true;
    setError("");
    setMode("sending");
    const release = beginOperation();
    try {
      const result = await post();
      if (!mounted.current) return;
      if (result.status === "sent") {
        setMode("sent");
        onSent();
        return;
      }
      setError(result.status === "failed" ? result.message : "Too many requests. Try again in a few minutes.");
    } catch {
      if (mounted.current) setError(failedMessage);
    } finally {
      inFlight.current = false;
      release();
    }
    if (!mounted.current) return;
    setMode("editing");
    onError?.();
  };

  return { mode, sending, error, setError, submit };
}

function FeedbackStatus({ mode, sent }: { mode: "editing" | "sending" | "sent"; sent: string }) {
  return <div role="status">{mode === "sending" ? "Sending…" : mode === "sent" ? sent : ""}</div>;
}

function CommandRequestRow({ original, onCancel, beginOperation }: { original: string; onCancel: () => void; beginOperation: () => () => void }) {
  const [open, setOpen] = useState(false);
  const { mode, sending, error, setError, submit } = useFeedbackSend(beginOperation, "Couldn't send the request. Try again.");
  const [command, setCommand] = useState(original);
  const [note, setNote] = useState("");
  const [commandInvalid, setCommandInvalid] = useState(false);
  const commandField = useRef<HTMLTextAreaElement>(null);

  const cancel = () => {
    setOpen(false);
    setCommand(original);
    setNote("");
    setError("");
    setCommandInvalid(false);
    onCancel();
  };

  const send = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (mode !== "editing") return;
    if (!command.trim() || command.trim().length > 300) {
      setError("Use 1–300 characters for the command.");
      setCommandInvalid(true);
      commandField.current?.focus();
      return;
    }
    void submit(
      () => api.sendCommandRequest(command, note.trim() ? note : undefined),
      onCancel,
      () => requestAnimationFrame(() => commandField.current?.focus()),
    );
  };

  if (!open) return (
    <div className="learn">
      <button type="button" className="primary-button request-command" onClick={() => setOpen(true)}>Request this command</button>
    </div>
  );

  return (
    <div className="learn command-request">
      {mode !== "sent" && (
        <form aria-label="Request this command" method="post" noValidate onSubmit={send} onKeyDown={(event) => {
          // Escape closes only this form, not the whole result.
          if (event.key !== "Escape" || event.nativeEvent.isComposing) return;
          event.preventDefault();
          if (!sending) cancel();
        }}>
          <p id="request-public">Your command and note will be posted publicly on GitHub. File and folder names will be visible, so remove any you want to keep private.</p>
          <label htmlFor="request-command">Command (required, up to 300 characters)</label>
          <textarea
            ref={commandField}
            className="path"
            id="request-command"
            name="command"
            rows={2}
            aria-required="true"
            autoFocus
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            value={command}
            readOnly={sending}
            aria-invalid={commandInvalid || undefined}
            aria-describedby="request-public request-error"
            onChange={(event) => {
              setCommand(event.target.value);
              setError("");
              setCommandInvalid(false);
            }}
          />
          <label htmlFor="request-note">Note (optional, up to 500 characters)</label>
          <textarea
            className="path"
            id="request-note"
            name="note"
            rows={2}
            maxLength={500}
            aria-describedby="request-public request-error"
            value={note}
            readOnly={sending}
            onChange={(event) => { setNote(event.target.value); setError(""); }}
          />
          <div id="request-error" className="warn" role="alert">{error}</div>
          <div className="learn-buttons">
            <button className="secondary small" type="submit" disabled={sending}>Send</button>
            <button className="link" type="button" disabled={sending} onClick={cancel}>Cancel</button>
          </div>
        </form>
      )}
      <FeedbackStatus mode={mode} sent="Request sent. Thanks!" />
    </div>
  );
}

/** Results that can be reported as wrong: anything Voxi decided, not a pick list. */
function reportable(p: Phase): boolean {
  return p.k === "done" || (p.k === "outcome" && p.outcome.status !== "ambiguous");
}

/** What the person saw, small enough to keep in a report. */
function reportOutcome(p: Phase): unknown {
  if (p.k === "done") return { status: "done", summary: p.result.summary, error: p.result.error };
  if (p.k !== "outcome") return { status: p.k };
  const o = p.outcome;
  switch (o.status) {
    case "plan": return { status: o.status, title: o.title, total: o.total, items: o.items.slice(0, 5).map((i) => `${i.kind}: ${i.primary} ${i.detail}`) };
    case "info": return { status: o.status, title: o.title, summary: o.summary, total: o.total, rows: o.rows.slice(0, 5).map((r) => r.name) };
    case "problem": return { status: o.status, message: o.message };
    default: return { status: o.status };
  }
}

/** Save a local report before offering a review of the public copy. */
function ReportRow({ command, phase, onDone, reviewing, onReviewing, beginOperation }: {
  command: string; phase: Phase; onDone: () => void; reviewing: boolean; onReviewing: (reviewing: boolean) => void; beginOperation: () => () => void;
}) {
  const [state, setState] = useState<{ k: "idle" } | { k: "writing"; note: string } | { k: "saving" } | { k: "saved"; count: number; note: string } | { k: "error"; message: string }>({ k: "idle" });
  const delivery = useFeedbackSend(beginOperation, "Couldn't send the report. Try again.");
  const { setError } = delivery;
  // A replaced result unmounts this row and any review it owns.
  useLayoutEffect(() => () => onReviewing(false), [onReviewing]);
  const closeReview = useCallback(() => {
    onReviewing(false);
    setError("");
  }, [onReviewing, setError]);
  if (!command) return null;
  const save = async (note: string) => {
    if (state.k !== "writing") return;
    setState({ k: "saving" });
    try {
      const searchStats = phase.k === "outcome" || phase.k === "done" ? phase.searchStats : undefined;
      const r = await api.reportWrong(command, reportOutcome(phase), note, searchStats);
      setState({ k: "saved", count: r.count, note });
    } catch (e) {
      setState({ k: "error", message: errorMessage(e) });
    }
    onDone();
  };
  if (state.k === "idle") return (
    <div className="learn">
      <button className="link" onClick={() => setState({ k: "writing", note: "" })}>Wrong result?</button>
    </div>
  );
  if (state.k === "saved") {
    const outcome = phase.k === "done" ? { status: "done", actionCounts: phase.actionCounts } :
      phase.k === "outcome" ? phase.outcome : { status: "problem" };
    return (
      <div className={reviewing ? "learn command-request wrong-result-review" : "learn report-saved"}>
        {reviewing ? (
          <WrongReportReview command={command} note={state.note} outcome={outcome} close={closeReview} delivery={delivery} />
        ) : (
          <>
            <span role="status">Saved on this Mac. {state.count} {state.count === 1 ? "report" : "reports"} so far.</span>
            {delivery.mode !== "sent" && <button type="button" className="secondary small" onClick={() => onReviewing(true)}>Review report to send</button>}
          </>
        )}
        <FeedbackStatus mode={delivery.mode} sent="Report sent. Thanks!" />
      </div>
    );
  }
  if (state.k === "error") return <div className="learn" role="alert"><span className="warn">{state.message}</span></div>;
  if (state.k === "saving") return <div className="learn" role="status"><span>Saving…</span></div>;
  return (
    <div className="learn writing">
      <input
        className="path"
        autoFocus
        value={state.note}
        placeholder="What should Voxi have done? (optional)"
        aria-label="What should Voxi have done?"
        onChange={(e) => setState({ k: "writing", note: e.target.value })}
        onKeyDown={(e) => {
          if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); save(state.note); }
        }}
      />
      <button className="secondary small" onClick={() => save(state.note)}>Save report</button>
      <button className="link" onClick={() => { setState({ k: "idle" }); onDone(); }}>Cancel</button>
    </div>
  );
}

function WrongReportReview({ command, note, outcome, close, delivery }: {
  command: string; note: string; outcome: ReportOutcome; close: () => void; delivery: ReturnType<typeof useFeedbackSend>;
}) {
  const [publicCommand, setPublicCommand] = useState(command);
  const [publicNote, setPublicNote] = useState(note);
  const [fieldErrors, setFieldErrors] = useState<WrongReportFieldErrors>({});
  const { actions, outcome: status } = reportFields(outcome);
  const { sending, error, setError, submit } = delivery;
  const commandField = useRef<HTMLTextAreaElement>(null);
  const noteField = useRef<HTMLTextAreaElement>(null);
  const errorRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || event.isComposing) return;
      event.preventDefault();
      if (!sending && !event.repeat) close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [close, sending]);

  const send = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (sending) return;
    const report = sanitizeWrongReport(publicCommand, outcome, publicNote, appVersion);
    if ("error" in report) {
      setFieldErrors(report.fields);
      setError("");
      const field = report.fields.command ? commandField : noteField;
      requestAnimationFrame(() => field.current?.focus());
      return;
    }
    setFieldErrors({});
    setPublicCommand(report.command);
    setPublicNote(report.note);
    const commandChanged = report.command !== publicCommand.trim();
    if (commandChanged || report.note !== publicNote.trim()) {
      setError("The text was normalized. Review the updated fields, then select Send this report again.");
      // Move off the button so a second press can't send the rewrite unseen.
      const changed = commandChanged ? commandField : noteField;
      requestAnimationFrame(() => changed.current?.focus());
      return;
    }
    void submit(() => api.sendWrongResult(report), close, () => requestAnimationFrame(() => errorRef.current?.focus()));
  };

  return (
    <form aria-labelledby="wrong-report-title" method="post" noValidate onSubmit={send}>
      <h2 id="wrong-report-title">Review report</h2>
      <p id="wrong-report-public">The fields below will be posted publicly on GitHub if you send this report. Remove private file, folder, user, and device names from the command and note. Edits apply to the public copy.</p>
      <label htmlFor="wrong-report-command">Command (required, up to {maxCommand} characters)</label>
      <textarea ref={commandField} id="wrong-report-command" name="command" className="path" rows={2} autoFocus aria-required="true"
        autoCorrect="off" autoCapitalize="off" spellCheck={false}
        aria-invalid={!!fieldErrors.command}
        aria-describedby={fieldErrors.command ? "wrong-report-public wrong-report-command-error" : "wrong-report-public"}
        value={publicCommand} readOnly={sending}
        onChange={(event) => {
          setPublicCommand(event.target.value);
          setFieldErrors((current) => ({ ...current, command: undefined }));
          setError("");
        }} />
      <label htmlFor="wrong-report-note">Note (optional, up to {maxNote} characters)</label>
      <textarea ref={noteField} id="wrong-report-note" name="note" className="path" rows={2}
        aria-invalid={!!fieldErrors.note}
        aria-describedby={fieldErrors.note ? "wrong-report-public wrong-report-note-error" : "wrong-report-public"}
        value={publicNote} readOnly={sending}
        onChange={(event) => {
          setPublicNote(event.target.value);
          setFieldErrors((current) => ({ ...current, note: undefined }));
          setError("");
        }} />
      <dl>
        <dt>Outcome</dt><dd>{status}</dd>
        <dt>Planned actions</dt><dd>{actions.map(({ kind, count }) => `${kind} ×${count}`).join(", ") || "None"}</dd>
        <dt>Voxi version</dt><dd>{appVersion}</dd>
      </dl>
      <div className="warn" role="alert" ref={errorRef} tabIndex={-1}>
        {fieldErrors.command && <div id="wrong-report-command-error">{fieldErrors.command}</div>}
        {fieldErrors.note && <div id="wrong-report-note-error">{fieldErrors.note}</div>}
        {error}
      </div>
      <div className="learn-buttons">
        <button className="primary-button" type="submit" disabled={sending}>Send this report</button>
        <button className="link" type="button" disabled={sending} onClick={close}>Cancel</button>
      </div>
    </form>
  );
}

/** One offer to learn from what just happened: a folder's nickname, or a
 *  phrase that should run the command that worked. */
// The answer lives in the parent's offer, not here: the row unmounts while a
// command runs, and an answered offer must not ask again when it comes back.
function OfferRow({ offer, onSaved, onDone }: { offer: Offer; onSaved: (s: Settings, text: string) => void; onDone: () => void }) {
  const [state, setState] = useState<{ k: "asking" } | { k: "error"; message: string }>({ k: "asking" });
  const accept = async () => {
    try {
      if (offer.k === "folder") {
        onSaved(await api.rememberFolder(offer.name, offer.path), `Saved. “${offer.name}” now means ${tildify(offer.path)}.`);
      } else {
        onSaved(await api.teachPhrase(offer.phrase, offer.command), `Saved. “${offer.phrase}” now runs “${offer.command}”. Change it under Shortcuts.`);
      }
    } catch (e) {
      setState({ k: "error", message: errorMessage(e) });
    }
  };
  if (offer.saved) return <div className="learn offer" role="status"><span>{offer.saved}</span></div>;
  if (state.k === "error") return <div className="learn offer" role="alert"><span className="warn">{state.message}</span></div>;
  const question = offer.k === "folder"
    ? <>Always use <strong>{tildify(offer.path)}</strong> for “{offer.name}”?</>
    : <>Next time you say “{offer.phrase}”, run this?</>;
  return (
    <div className="learn offer">
      <span>{question}</span>
      <span className="learn-buttons">
        <button className="secondary small" onClick={accept}>{offer.k === "folder" ? "Remember" : "Teach Voxi"}</button>
        <button className="link" onClick={onDone}>No thanks</button>
      </span>
    </div>
  );
}

function phaseKey(p: Phase): string {
  if (p.k !== "outcome") return p.k;
  const o = p.outcome;
  return `outcome-${o.status}-${o.status === "plan" ? o.id : o.status === "ambiguous" ? o.name : ""}`;
}


function Tick() {
  return (
    <svg className="tick" viewBox="0 0 20 20" aria-hidden="true">
      <path d="M4 10.5l4 4 8-9" />
    </svg>
  );
}

type PlanOutcome = Extract<Outcome, { status: "plan" }>;

/** The preview's steps. The first page arrives with the plan; "Show more"
 *  loads the rest of the same frozen plan, and the notes view lists only the
 *  steps with a note (a name changed to avoid a clash, say). */
/** The results label Voxi gives files sent from Finder; matches finder_selection::LABEL. */
const FINDER_LABEL = "your Finder selection";

const runLabel = (n: number) => `Run ${n} ${n === 1 ? "change" : "changes"}`;

function PlanSteps({ plan, skipped, onToggle }: { plan: PlanOutcome; skipped: number[]; onToggle: (step: number) => void }) {
  const first = { notes: false, before: 0, rows: plan.items.map((it, step): PreviewRow => ({ ...it, step })) };
  // One page at a time, so a huge plan never mounts every row. `before` is
  // how many rows of this view came on earlier pages; `back` holds them.
  const [page, setPage] = useState<{ notes: boolean; before: number; rows: PreviewRow[] }>(first);
  const [back, setBack] = useState<{ before: number; rows: PreviewRow[] }[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const prevRef = useRef<HTMLButtonElement>(null);
  const nextRef = useRef<HTMLButtonElement>(null);
  const last = page.rows[page.rows.length - 1] as PreviewRow | undefined;
  const seen = page.before + page.rows.length;
  const hasNext = page.notes ? seen < plan.noted : (last ? last.step + 1 : 0) < plan.total;
  const show = (next: { notes: boolean; before: number; rows: PreviewRow[] }) => {
    const nextHadFocus = document.activeElement === nextRef.current;
    setPage(next);
    listRef.current?.scrollTo({ top: 0 });
    // The Next button goes away on the last page: keep focus nearby.
    if (nextHadFocus) requestAnimationFrame(() => (nextRef.current ?? prevRef.current)?.focus());
  };
  const load = (notes: boolean, offset: number, before: number, onDone?: () => void) => {
    setLoading(true);
    setError(null);
    api.previewPage(plan.id, offset, notes)
      .then((rows) => { onDone?.(); show({ notes, before, rows }); })
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setLoading(false));
  };
  const next = () => load(page.notes, last ? last.step + 1 : 0, seen, () => setBack((b) => [...b, { before: page.before, rows: page.rows }]));
  const previous = () => {
    const prev = back[back.length - 1];
    if (!prev) return;
    setBack((b) => b.slice(0, -1));
    show({ notes: page.notes, ...prev });
  };
  // Switching views starts a new page history, but only once the new page
  // arrives: a failed load keeps the current page and its Previous.
  const toggleNotes = () => {
    if (page.notes) {
      setBack([]);
      show(first);
    } else load(true, 0, 0, () => setBack([]));
  };
  const total = page.notes ? plan.noted : plan.total;
  const paged = plan.total > plan.items.length || plan.noted > 0;
  return (
    <>
      <ul ref={listRef} className={`ruled scroll${page.notes ? " numbered" : ""}`} aria-label={page.notes ? "Steps with notes" : "Steps"}>
        {page.rows.map((it, i) => (
          <li key={it.step} data-kind={it.kind} data-skipped={skipped.includes(it.step) || undefined} style={row(i)}>
            {page.notes && <span className="num"><span className="sr-only">Step </span>{it.step + 1}</span>}
            <span className="primary">
              {it.skippable && (
                <input
                  type="checkbox"
                  className="keep"
                  checked={!skipped.includes(it.step)}
                  onChange={() => onToggle(it.step)}
                  aria-label={`Include ${it.primary}`}
                />
              )}
              {it.primary}
            </span>
            <span className="detail">{it.detail}</span>
            {it.note && <span className="note">{it.note}</span>}
          </li>
        ))}
      </ul>
      {paged && (
        <p className="quiet pager">
          {page.rows.length > 0 && (total > page.rows.length || back.length > 0) && (
            <span>
              {page.notes ? "With notes: " : "Steps "}
              {page.before + 1}–{seen} of {total}
            </span>
          )}
          {back.length > 0 && <button ref={prevRef} type="button" className="link" disabled={loading} onClick={previous}>Previous</button>}
          {hasNext && (
            <button ref={nextRef} type="button" className="link" disabled={loading} onClick={next}>
              {loading ? "Loading…" : `Next ${Math.min(PREVIEW_PAGE, total - seen)}`}
            </button>
          )}
          {plan.noted > 0 && (
            <button type="button" className="link" disabled={loading} onClick={toggleNotes}>
              {page.notes ? "All steps" : `Only the ${plan.noted} with notes`}
            </button>
          )}
          {error && <span className="warn" role="alert">{error}</span>}
        </p>
      )}
    </>
  );
}

/** Steps each later page holds; matches PREVIEW_PAGE in lib.rs. */
const PREVIEW_PAGE = 200;

function Body(props: {
  searched: number;
  phase: Phase;
  asking: Asking | null;
  heard: string | null;
  onConfirm: () => void;
  onCancel: () => void;
  onStop: () => void;
  onPick: (index: number) => void;
  onFill: (s: string) => void;
  onUndo: () => void;
  skipped: number[];
  onToggleSkip: (id: number, step: number) => void;
}) {
  const { phase } = props;
  if (phase.k === "ready") return null;
  if (phase.k === "parsing") return (
    <section className="body" aria-live="polite">
      <p className="quiet">Searching… {props.searched.toLocaleString()} entries checked.</p>
      <div className="actions"><button className="secondary" onClick={props.onCancel}>Cancel</button></div>
    </section>
  );

  if (phase.k === "running") {
    return (
      <section className="body">
        <p className="title">{phase.title}</p>
        <p className="quiet" role="status">{phase.stopping ? "Stopping…" : "Working…"}</p>
        {phase.stopError && <p className="note warn" role="alert">{phase.stopError}</p>}
        {phase.canStop && <div className="actions">
          <button type="button" className="secondary stop-execution" disabled={phase.stopping} onClick={props.onStop}>
            Stop <kbd>esc</kbd>
          </button>
        </div>}
      </section>
    );
  }

  if (phase.k === "error") {
    return (
      <section className="body">
        <p className="title warn nudge">{phase.message}</p>
      </section>
    );
  }

  if (phase.k === "done") {
    const r = phase.result;
    return (
      <section className="body">
        <p className={r.error ? "title warn nudge" : "title"} role="status">
          {!r.error && !r.cancelled && <Tick />}
          {r.summary}
        </p>
        {r.error && <p className="note">{r.error}</p>}
        <div className="actions">
          {r.undoable && (
            <button className="secondary" onClick={props.onUndo}>
              Undo
            </button>
          )}
        </div>
      </section>
    );
  }

  const o = phase.outcome;
  switch (o.status) {
    case "not_understood":
      return (
        <section className="body">
          <p className="title nudge">Didn't understand: “{o.text}”</p>
          <p className="quiet">Use a filename, folder, or a specific group of files. For example:</p>
          <ExampleList onFill={props.onFill} />
        </section>
      );
    case "problem":
      return (
        <section className="body">
          <p className="title warn nudge">{o.message}</p>
        </section>
      );
    case "ambiguous":
      return (
        <section className="body">
          <p className="title">{o.candidates.length === 1 ? "Confirm the match" : "Choose a match"} for “{o.name}”</p>
          {o.reason === "incomplete_search" && <p className="quiet">{o.note ?? "Deeper folders weren’t fully checked."}</p>}
          <p className="quiet">Say a number, or click one.</p>
          <ol className="ruled choices">
            {o.candidates.map((c, i) => (
              <li key={c} style={row(i)}>
                <button className="row-button" onClick={() => props.onPick(i)} aria-label={`${i + 1}: ${c}`}>
                  <span className="num" aria-hidden="true">
                    {i + 1}
                  </span>
                  <span className="primary">{c.split("/").pop()}</span>
                  <span className="detail">{tildify(c.slice(0, c.lastIndexOf("/")))}</span>
                </button>
              </li>
            ))}
          </ol>
          {props.heard && (
            <p className="quiet nudge" key={props.heard}>
              Heard “{props.heard}”. Say a number from 1 to {o.candidates.length}, or “cancel”.
            </p>
          )}
        </section>
      );
    case "info":
      return (
        <section className="body">
          <p className="title">{o.title}</p>
          {o.summary && <p className="quiet">{o.summary}</p>}
          {o.rows.length > 0 && (
            <ol className={`ruled scroll${o.numbered ? " numbered" : ""}`}>
              {o.rows.map((r, i) => (
                <li key={i} style={row(i)}>
                  {o.numbered && <span className="num" aria-hidden="true">{i + 1}</span>}
                  <span className="primary">{r.name}</span>
                  <span className="detail">{r.detail}</span>
                </li>
              ))}
            </ol>
          )}
          {o.total > o.rows.length && o.rows.length > 0 && <p className="quiet">Showing {o.rows.length} of {o.total}.</p>}
        </section>
      );
    case "plan": {
      return (
        <section className="body">
          <p className="title">{o.title}</p>
          {phase.refused && <p className="note warn" role="alert">{phase.refused}</p>}
          {o.modelInterpreted && (
            <p className="model-notice">
              <strong>Model interpretation.</strong>{" "}
              {o.scheduledFor ? "Check the file changes and schedule before confirming." : "Check the file changes before confirming."}
            </p>
          )}
          <PlanSteps plan={o} skipped={props.skipped} onToggle={(step) => props.onToggleSkip(o.id, step)} />
          {props.skipped.length > 0 && (
            <p className="quiet" role="status">
              Leaving out {props.skipped.length} {props.skipped.length === 1 ? "step" : "steps"}.
              {props.skipped.length >= o.total && " Tick at least one to run."}
            </p>
          )}
          {o.firstRun === "runNowAndRepeat" && <p className="quiet">{o.repeats}</p>}
          {o.scopeNotice && <p className="quiet">{o.scopeNotice}</p>}
          {o.warnings.map((warning, index) => (
            <p key={`${index}-${warning}`} className="quiet warn">
              {warning}
            </p>
          ))}
          <div className="actions">
            <button className="primary-button" onClick={props.onConfirm} disabled={props.skipped.length >= o.total}>
              {o.firstRun === "runNowAndRepeat" ? "Run now and repeat" : o.firstRun === "repeatLater" ? "Schedule repeating task" : o.firstRun === "oneTimeDeferred" ? "Schedule task" : o.isUndo ? "Undo" : runLabel(o.total - props.skipped.length)}
              <kbd>↩</kbd>
            </button>
            <button className="secondary" onClick={props.onCancel}>
              Cancel <kbd>esc</kbd>
            </button>
          </div>
        </section>
      );
    }
  }
}

function Footer({ phase, asking, onSettings }: { phase: Phase; asking: boolean; onSettings: () => void }) {
  const hint = asking
    ? "Enter to choose. Esc to cancel."
    : phase.k === "parsing"
      ? "Reading…"
      : phase.k === "ready"
        ? "Enter to preview. Shift-Enter for a new line."
        : phase.k === "outcome" && phase.outcome.status === "plan"
          ? ({
            runNowAndRepeat: "Enter to run now and repeat. Later runs won't ask first. Esc to cancel.",
            repeatLater: "Enter to schedule. Later runs won't ask first. Esc to cancel.",
            oneTimeDeferred: "Enter to schedule. Esc to cancel.",
            immediate: "Enter to run. Nothing changes until then.",
          }[phase.outcome.firstRun])
          : phase.k === "outcome" && phase.outcome.status === "ambiguous"
            ? "Esc to go back"
            : phase.k === "running" ? "" : "Say the next command. Esc to go back.";
  return (
    <footer className="foot">
      <span>{hint}</span>
      <button className="link" onClick={onSettings} disabled={phase.k === "running"} title="Settings (⌘,)">
        Settings <kbd>⌘,</kbd>
      </button>
    </footer>
  );
}

// ---- Shared frame ------------------------------------------------------------

/** Every screen uses the command window's layout: a title on the red rule,
 *  the content, and a quiet footer line. */

// ---- Settings ----------------------------------------------------------------
