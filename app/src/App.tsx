import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { api, errorMessage, inTauri, tildify, type Outcome, type ResultScope, type RunResult, type Settings } from "./api";
import { readChoice } from "./choice";
import examples from "./examples.json";
import { AccessView } from "./access";
import { SettingsView } from "./settings";
import { MAX_COMMAND_BYTES, TOO_LONG, row, utf8Bytes } from "./ui";

type View = "command" | "settings" | "welcome";
type Phase =
  | { k: "ready" }
  | { k: "parsing" }
  | { k: "outcome"; outcome: Outcome }
  | { k: "running"; title: string; canStop: boolean; stopping: boolean; stopError?: string }
  | { k: "done"; result: RunResult; wasUndo: boolean }
  | { k: "error"; message: string };

/** Waiting for the answer to "which one?": the command being planned and
 *  the picks made so far (one command can need several). */
type Asking = { command: string; choices: Record<string, string>; name: string; candidates: string[]; folders: boolean };

/** Something Voxi can learn from what just happened, offered once. */
type Offer =
  | { k: "folder"; name: string; path: string }
  | { k: "phrase"; phrase: string; command: string };

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
  const [settings, setSettings] = useState<Settings | null>(null);
  const [open, setOpen] = useState(!inTauri);
  const [pendingShow, setPendingShow] = useState<number | null>(null);
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
  const shownAt = useRef(0);
  useEffect(() => {
    if (!inTauri) return;
    const onBlur = () => {
      if (busyRef.current) return;
      cancelBlurClose();
      const requestId = showId.current;
      // WebKit can briefly blur during activation, paste, or text services.
      // A blur inside the opening grace still needs a deferred check: wait
      // for the grace to end, then for focus to settle, before closing.
      const graceLeft = Math.max(0, 300 - (performance.now() - shownAt.current));
      blurTimer.current = setTimeout(() => {
        blurTimer.current = null;
        if (document.hasFocus() || busyRef.current || requestId !== showId.current
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
  }, [close, cancelBlurClose]);

  useEffect(() => {
    api.getSettings().then((s) => {
      setSettings(s);
      if (!s.onboarded) setView("welcome");
    });
    if (!inTauri) return;
    let disposed = false;
    const unShown = listen<{ requestId: number; mode: View }>("voxi://shown", (e) => {
      const { mode, requestId } = e.payload;
      cancelClose();
      showId.current = requestId;
      awaitingShow.current = requestId;
      // Keep the confirmation and Stop control if the window is reopened mid-run.
      if (!executingRef.current) {
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
      if (disposed || requestId !== showId.current || awaitingShow.current !== null || busyRef.current) return;
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
  }, [close, cancelClose]);

  useEffect(() => {
    let disposed = false;
    document.fonts.ready.then(() => { if (!disposed) setFontsReady(true); });
    return () => { disposed = true; };
  }, []);

  // Layout is measured after settings and fonts settle, while the native window
  // is still hidden. The acknowledgement resizes and reveals it in one operation.
  useLayoutEffect(() => {
    if (!inTauri || pendingShow === null || !settings || !fontsReady || !innerRef.current) return;
    const requestId = pendingShow;
    const height = Math.ceil(innerRef.current.offsetHeight);
    void invoke("present_window", { requestId, height, animate: !reducedMotion() });
  }, [pendingShow, settings, fontsReady, view]);

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
            open={open}
            onClose={close}
            onBusy={setBusy}
            onSettings={() => setView("settings")}
          />}
        {view === "settings" && settings && (
          <SettingsView
            settings={settings}
            onSaved={setSettings}
            onClose={() => setView("command")}
            setBusy={setBusy}
          />
        )}
        {view === "welcome" && settings && (
          <AccessView
            settings={settings}
            firstRun
            setBusy={setBusy}
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
 *  person would, while the box is empty. */
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
  open,
  onClose,
  onBusy,
  onSettings,
}: {
  open: boolean;
  onClose: () => void;
  onBusy: (busy: boolean, executing?: boolean) => void;
  onSettings: () => void;
}) {
  const [text, setText] = useState("");
  const [phase, setPhase] = useState<Phase>({ k: "ready" });
  const [asking, setAsking] = useState<Asking | null>(null);
  // The last command, echoed above its result once the box is cleared.
  const [said, setSaid] = useState<string | null>(null);
  const [heard, setHeard] = useState<string | null>(null);
  const [resultScope, setResultScope] = useState<ResultScope | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const ruleRef = useRef<HTMLSpanElement>(null);
  const planShownAt = useRef(0);
  const activeRun = useRef<{ id: number; canStop: boolean; stopping: boolean } | null>(null);
  const requestId = useRef(0);
  const [searched, setSearched] = useState(0);
  // The command behind the result on screen, exactly as entered.
  const lastCommand = useRef("");
  // The last phrase Voxi didn't understand, which the next run can teach.
  const missed = useRef<{ text: string; at: number } | null>(null);
  const [offer, setOffer] = useState<Offer | null>(null);
  // A folder picked for an ambiguous name, offered as a nickname after the plan.
  const pickedFolder = useRef<{ name: string; path: string } | null>(null);
  // Same bound the backend applies to every command; checked before trimming.
  const tooLong = utf8Bytes(text) > MAX_COMMAND_BYTES;

  useEffect(() => () => {
    ++requestId.current;
    api.cancel();
    void api.clearResultScope();
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
      if (!el) return;
      el.focus({ preventScroll: true });
      el.setSelectionRange(el.value.length, el.value.length);
    });
  }, []);

  // Each native show request mounts a fresh command session before layout is
  // measured. Focus events only focus the input; they do not reset a live session.
  useEffect(() => {
    window.addEventListener("voxi-focus", focusInput);
    window.addEventListener("focus", focusInput);
    focusInput();
    return () => {
      window.removeEventListener("voxi-focus", focusInput);
      window.removeEventListener("focus", focusInput);
    };
  }, [focusInput]);

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
    async (command: string, picked: Record<string, string> = {}) => {
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
      const request = ++requestId.current;
      setSearched(0);
      setPhase({ k: "parsing" });
      setHeard(null);
      try {
        const { outcome, scope } = await api.plan(command, picked, request);
        if (request !== requestId.current) return;
        setResultScope(scope);
        setPhase({ k: "outcome", outcome });
        if (outcome.status === "plan") planShownAt.current = performance.now();
        if (outcome.status === "not_understood") missed.current = { text: command.trim(), at: Date.now() };
        // Once the picked folder led somewhere, offer to remember it by name.
        if (folderPick && (outcome.status === "plan" || outcome.status === "info")) setOffer({ k: "folder", ...folderPick });
        if (outcome.status === "ambiguous") {
          // The answer will be dictated too.
          setAsking({ command, choices: picked, name: outcome.name, candidates: outcome.candidates, folders: outcome.folders });
        } else {
          setAsking(null);
        }
        // Focus stays in the box for every outcome, previews included, so
        // the next thing dictated always lands there. Enter runs a preview.
        focusInput();
      } catch (e) {
        if (request === requestId.current) setPhase({ k: "error", message: errorMessage(e) });
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
      submit(asking.command, { ...asking.choices, [asking.name]: path });
    },
    [asking, submit],
  );

  const stopAsking = useCallback(() => {
    setAsking(null);
  }, []);

  const confirm = useCallback(async () => {
    if (activeRun.current || phase.k !== "outcome" || phase.outcome.status !== "plan") return;
    const plan = phase.outcome;
    const canStop = !plan.scheduledFor || !!plan.repeats?.startsWith("Runs now, then ");
    activeRun.current = { id: plan.id, canStop, stopping: false };
    setPhase({ k: "running", title: plan.title, canStop, stopping: false });
    try {
      const result = await api.confirm(plan.id);
      setResultScope(await api.resultScope().catch(() => null));
      setPhase({ k: "done", result, wasUndo: plan.isUndo });
      // A phrase Voxi missed, followed soon by a command that worked, can
      // be taught: next time the phrase runs this command.
      const miss = missed.current;
      missed.current = null;
      if (!result.error && !result.cancelled && !plan.isUndo && miss && Date.now() - miss.at < TEACH_WINDOW_MS && miss.text !== lastCommand.current) {
        setOffer({ k: "phrase", phrase: miss.text, command: lastCommand.current });
      }
      focusInput();
    } catch (e) {
      setPhase({ k: "error", message: errorMessage(e) });
    } finally {
      activeRun.current = null;
    }
  }, [phase, focusInput]);

  const stop = useCallback(async () => {
    const run = activeRun.current;
    if (!run?.canStop || run.stopping) return;
    run.stopping = true;
    setPhase((current) => current.k === "running" ? { ...current, stopping: true, stopError: undefined } : current);
    try {
      await api.stopExecution(run.id);
    } catch (e) {
      if (activeRun.current !== run) return;
      run.stopping = false;
      setPhase((current) => current.k === "running" ? { ...current, stopping: false, stopError: errorMessage(e) } : current);
    }
  }, []);

  const reset = useCallback(() => {
    const request = ++requestId.current;
    void Promise.all([api.cancel(), api.clearResultScope()]).catch((e) => {
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
      if (e.defaultPrevented || e.isComposing) return;
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
  }, [phase, text, resultScope, reset, stop, onSettings, onClose]);

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
    setAsking(null);
    submit("undo last", {});
  };

  const executing = phase.k === "running";
  const busy = phase.k === "parsing" || executing;
  useLayoutEffect(() => {
    onBusy(busy, executing);
    return () => onBusy(false);
  }, [busy, executing, onBusy]);
  const placeholder = useTypedPlaceholder(open && phase.k === "ready" && text === "");

  return (
    <>
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
        <span>{resultScope.count} results from {resultScope.label}{resultScope.visible < resultScope.count ? ` · ${resultScope.visible} displayed` : ""}</span>
        <button className="link" disabled={phase.k === "running"} onClick={() => {
          reset();
          void api.clearResultScope().then(() => setResultScope(null)).catch((e) => setPhase({ k: "error", message: errorMessage(e) }));
        }}>Clear scope</button>
      </div>}
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
      />
      {!busy && offer && <OfferRow key={offer.k === "folder" ? offer.path : offer.phrase} offer={offer} onDone={() => { setOffer(null); focusInput(); }} />}
      {reportable(phase) && <ReportRow key={`report-${phaseKey(phase)}`} command={lastCommand.current} phase={phase} onDone={focusInput} />}
      <Footer phase={phase} asking={!!asking} onSettings={onSettings} />
    </>
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

/** "Wrong result?": saves the command and what Voxi did with it on this Mac,
 *  with an optional note on what it should have done. Nothing is sent. */
function ReportRow({ command, phase, onDone }: { command: string; phase: Phase; onDone: () => void }) {
  const [state, setState] = useState<{ k: "idle" } | { k: "writing"; note: string } | { k: "saved"; count: number } | { k: "error"; message: string }>({ k: "idle" });
  if (!command) return null;
  const save = async (note: string) => {
    try {
      const r = await api.reportWrong(command, reportOutcome(phase), note);
      setState({ k: "saved", count: r.count });
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
  if (state.k === "saved") return <div className="learn" role="status"><span>Saved on this Mac. Nothing is sent. {state.count} {state.count === 1 ? "report" : "reports"} so far.</span></div>;
  if (state.k === "error") return <div className="learn" role="alert"><span className="warn">{state.message}</span></div>;
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

/** One offer to learn from what just happened: a folder's nickname, or a
 *  phrase that should run the command that worked. */
function OfferRow({ offer, onDone }: { offer: Offer; onDone: () => void }) {
  const [state, setState] = useState<{ k: "asking" } | { k: "saved"; text: string } | { k: "error"; message: string }>({ k: "asking" });
  const accept = async () => {
    try {
      if (offer.k === "folder") {
        await api.rememberFolder(offer.name, offer.path);
        setState({ k: "saved", text: `Saved. “${offer.name}” now means ${tildify(offer.path)}.` });
      } else {
        await api.teachPhrase(offer.phrase, offer.command);
        setState({ k: "saved", text: `Saved. “${offer.phrase}” now runs “${offer.command}”. Change it under Shortcuts.` });
      }
    } catch (e) {
      setState({ k: "error", message: errorMessage(e) });
    }
  };
  if (state.k === "saved") return <div className="learn offer" role="status"><span>{state.text}</span></div>;
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
          <p className="title">Choose a match for “{o.name}”</p>
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
      const more = o.total - o.items.length;
      return (
        <section className="body">
          <p className="title">{o.title}</p>
          <ul className="ruled scroll">
            {o.items.map((it, i) => (
              <li key={i} data-kind={it.kind} style={row(i)}>
                <span className="primary">{it.primary}</span>
                <span className="detail">{it.detail}</span>
                {it.note && <span className="note">{it.note}</span>}
              </li>
            ))}
            {more > 0 && (
              <li className="more" style={row(o.items.length)}>
                and {more} more {more === 1 ? "step" : "steps"}
              </li>
            )}
          </ul>
          {o.repeats?.startsWith("Runs now, then ") && <p className="quiet">{o.repeats}</p>}
          {o.warnings.map((w) => (
            <p key={w} className="quiet">
              {w}
            </p>
          ))}
          <div className="actions">
            <button className="primary-button" onClick={props.onConfirm}>
              {o.repeats?.startsWith("Runs now, then ") ? "Run now and repeat" : o.repeats ? "Schedule repeating task" : o.scheduledFor ? "Schedule task" : o.isUndo ? "Undo" : `Run ${o.total} ${o.total === 1 ? "change" : "changes"}`}
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
          ? (phase.outcome.scheduledFor ? (phase.outcome.repeats?.startsWith("Runs now, then ") ? "Enter to run now and repeat. Later runs won't ask first. Esc to cancel." : phase.outcome.repeats ? "Enter to schedule. Later runs won't ask first. Esc to cancel." : "Enter to schedule. Esc to cancel.") : "Enter to run. Nothing changes until then.")
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
