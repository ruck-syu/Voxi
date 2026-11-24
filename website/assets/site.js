// Points the download buttons at the newest DMG, copies the Homebrew command,
// types the hero sentence and its plan, runs the live demo (the real Voxi frontend in /demo/, on its sample backend,
// with the menu bar face talking as it does in the app), and
// the command search.

const REPO = "ruck-syu/Voxi";
const reduced = matchMedia("(prefers-reduced-motion: reduce)");
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

// ---- Download: newest release's DMG ------------------------------------------

(async () => {
  const links = document.querySelectorAll("[data-download]");
  const labels = document.querySelectorAll("[data-version]");
  if (!links.length && !labels.length) return;
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) return; // The buttons already link to the releases page.
    const release = await res.json();
    const dmg = release.assets?.find((a) => a.name.endsWith(".dmg"));
    if (dmg) links.forEach((a) => (a.href = dmg.browser_download_url));
    const version = String(release.tag_name || "").replace(/^v/, "");
    if (version)
      labels.forEach(
        (el) =>
          (el.textContent = `Version ${version}. Free and open source, for macOS 12 or later.`),
      );
  } catch {
    // Offline or rate-limited: keep the releases-page link.
  }
})();

// ---- Copy the Homebrew install command ---------------------------------------

document.querySelectorAll("[data-copy-command]").forEach((panel) => {
  const command = panel.querySelector("[data-copy-text]");
  const button = panel.querySelector("[data-copy-button]");
  const status = panel.querySelector("[data-copy-status]");
  if (!command || !button || !status) return;
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(command.textContent.trim());
      button.textContent = "Copied";
      status.textContent = "Homebrew install command copied.";
    } catch {
      button.textContent = "Copy failed";
      status.textContent = "Copy failed. Select the command and copy it.";
    }
    setTimeout(() => (button.textContent = "Copy"), 1800);
  });
});

// ---- Light and dark mode -------------------------------------------------------
// Dark by default (set on <html>). The switch flips it and remembers the choice in this
// browser; the <head> applies a saved choice before the page paints. The demo
// iframe takes its colour scheme from its `color-scheme`, set in the CSS.

(() => {
  const root = document.documentElement;
  const switches = document.querySelectorAll("[data-theme-switch]");
  const meta = document.querySelector('meta[name="theme-color"]');
  const sync = () => {
    const dark = root.dataset.theme === "dark";
    switches.forEach((b) =>
      b.setAttribute("aria-label", `Switch to ${dark ? "light" : "dark"} mode`),
    );
    if (meta) meta.content = dark ? "#0f1a36" : "#eef3fa";
  };
  switches.forEach((button) =>
    button.addEventListener("click", () => {
      const dark = root.dataset.theme !== "dark";
      if (dark) root.dataset.theme = "dark";
      else delete root.dataset.theme;
      dispatchEvent(new Event("voxi-theme"));
      try {
        localStorage.setItem("voxi-theme", dark ? "dark" : "light");
      } catch {
        // Storage blocked: the choice lasts for this page only.
      }
      sync();
    }),
  );
  sync();
})();

// ---- The menu bar face ---------------------------------------------------------
// Drawn exactly as docs/icon/appicon.swift draws the 88px tray template: two
// eyes and a smile that is a sound wave, y = 62 + 8s + ripple·sin(6πt − phase)·s
// with s = sin(πt). Resting: ripple 2.6. Talking: ripple 4.2, and twelve frames
// a twelfth of a wave apart, 50ms each, lingering 450ms after the last
// keystroke and finishing the sweep, as app/src-tauri/src/tray_face.rs plays it.

const Tray = (() => {
  const k = 1.45;
  const p = (x, y) => [44 + (x - 50) * k, 44 + (y - 51) * k];
  const r2 = (v) => Math.round(v * 100) / 100;
  const smile = (ripple, phase) => {
    let d = "";
    for (let i = 0; i <= 64; i++) {
      const t = i / 64;
      const s = Math.sin(Math.PI * t);
      const [x, y] = p(26 + 48 * t, 62 + 8 * s + ripple * Math.sin(6 * Math.PI * t - phase) * s);
      d += `${i ? "L" : "M"}${r2(x)} ${r2(y)}`;
    }
    return d;
  };
  const eyes = [31, 61]
    .map((x) => {
      const [ax, ay] = p(x, 26);
      const [bx, by] = p(x + 9, 46);
      const r = (9 / 2) * k;
      return `<rect class="eye" x="${r2(ax)}" y="${r2(ay)}" width="${r2(bx - ax)}" height="${r2(by - ay)}" rx="${r2(r)}"/>`;
    })
    .join("");
  const REST = smile(2.6, 0);
  const FRAMES = Array.from({ length: 12 }, (_, i) => smile(4.2, (i * Math.PI) / 6));
  const FRAME = 50;
  const LINGER = 450;

  const el = document.querySelector("[data-tray]");
  let path = null;
  if (el) {
    el.innerHTML = `<svg viewBox="0 0 88 88" aria-hidden="true">${eyes}<path class="smile" stroke-width="${7.5 * k}" d="${REST}"/></svg>`;
    path = el.querySelector("path");
  }

  let running = false;
  let last = 0;
  async function nudge() {
    if (!path || reduced.matches) return;
    last = performance.now();
    if (running) return;
    running = true;
    let i = 0;
    for (;;) {
      const idle = performance.now() - last >= LINGER;
      if (idle && i % FRAMES.length === 0) break;
      path.setAttribute("d", FRAMES[i % FRAMES.length]);
      i++;
      await wait(FRAME);
    }
    path.setAttribute("d", REST);
    running = false;
    if (performance.now() - last < LINGER) nudge();
  }
  return { nudge };
})();

// ---- Hero: a sentence typed out, understood phrase by phrase, then its plan ----
// The page ships the first scene already finished, which is what people with
// reduced motion (or no JavaScript) see. Otherwise the scenes type themselves
// while the stage is on screen.

(() => {
  const stage = document.querySelector("[data-stage]");
  if (!stage || reduced.matches) return;
  const said = stage.querySelector("[data-said]");
  const plan = stage.querySelector("[data-plan]");
  const title = stage.querySelector("[data-plan-title]");
  const rowsEl = stage.querySelector("[data-plan-rows]");
  const foot = stage.querySelector("[data-plan-foot]");
  const desc = stage.querySelector("[data-stage-status]");

  const LABEL = {
    do: "action", what: "which files", how: "how", as: "new format", where: "in",
    from: "from", to: "to", when: "when",
  };
  const TAG = { how: "what", as: "what", where: "from" };
  const tagOf = (kind) => TAG[kind] || kind;
  const DOC = '<path d="M5 2h7l4 4v12H5Z"/><path d="M12 2v4h4"/>';
  const DIR = '<path d="M2 5.5h6l1.5 2H18V16H2Z"/>';
  // A row's `from` is optional: renames and conversions show old → new.
  const scenes = [
    {
      parts: [["do", "move"], ["what", "PDFs"], [null, "from"], ["from", "Downloads"], [null, "to"], ["to", "Documents"]],
      title: "3 files will move",
      rows: [
        { file: "invoice-april.pdf", from: "Downloads", to: "Documents" },
        { file: "boarding-pass.pdf", from: "Downloads", to: "Documents" },
        {
          file: "lease.pdf",
          from: "Downloads",
          to: "Documents",
          note: "Documents already has a lease.pdf, so this one becomes <b>lease 2.pdf</b>.",
        },
      ],
      foot: "Nothing else in Downloads is touched.",
    },
    {
      parts: [["do", "sort"], ["from", "Downloads"], ["how", "by type"], ["when", "every Monday at 8 am"]],
      title: "Repeats every Monday at 8:00 AM",
      rows: [
        { file: "12 images", dir: true, from: "Downloads", to: "Images" },
        { file: "7 PDFs", dir: true, from: "Downloads", to: "PDFs" },
        { file: "3 videos", dir: true, from: "Downloads", to: "Videos" },
      ],
      foot: "Each run finds that week's files, and every run can be undone from History.",
    },
    {
      parts: [["do", "convert"], ["what", "HEIC"], [null, "in"], ["where", "Pictures"], [null, "to"], ["as", "jpg"]],
      title: "3 JPG copies will be made",
      rows: [
        { file: "IMG_4031.HEIC", to: "IMG_4031.jpg" },
        { file: "IMG_4032.HEIC", to: "IMG_4032.jpg" },
        { file: "beach-day.HEIC", to: "beach-day.jpg" },
      ],
      foot: "The originals stay as they are, next to their copies.",
    },
    {
      parts: [["do", "clean up"], ["from", "Downloads"], ["what", "older than 90 days"]],
      title: "4 files will go to the Trash",
      rows: [
        { file: "Zoom-installer.pkg", from: "Downloads", to: "Trash" },
        { file: "IMG_2231.HEIC", from: "Downloads", to: "Trash" },
        { file: "old-resume-v3.docx", from: "Downloads", to: "Trash" },
        { file: "Figma-agent.dmg", from: "Downloads", to: "Trash" },
      ],
      foot: "The Trash, not erased. Tell it “undo last” to bring them all back.",
    },
    {
      parts: [["do", "replace"], ["how", "spaces with dashes"], [null, "in"], ["what", "files"], [null, "in"], ["where", "Downloads"]],
      title: "3 files will be renamed",
      rows: [
        { file: "Q3 report final.pdf", to: "Q3-report-final.pdf" },
        { file: "team photo.png", to: "team-photo.png" },
        { file: "notes from call.txt", to: "notes-from-call.txt" },
      ],
      foot: "Files stay in their folders. Only the names change.",
    },
    {
      parts: [["do", "merge"], ["what", "PDFs"], [null, "in"], ["where", "Documents"], ["what", "named scan"], [null, "into"], ["to", "scan-all.pdf"]],
      title: "3 PDFs will become one",
      rows: [
        { file: "scan-01.pdf", from: "Documents", to: "scan-all.pdf" },
        { file: "scan-02.pdf", from: "Documents", to: "scan-all.pdf" },
        { file: "scan-03.pdf", from: "Documents", to: "scan-all.pdf" },
      ],
      foot: "Pages go in name order. The three originals are kept.",
    },
  ];

  function renderPlan(scene) {
    title.textContent = scene.title;
    rowsEl.innerHTML = scene.rows
      .map(
        (r, i) =>
          `<li style="animation-delay:${i * 70}ms"><p class="file"><svg viewBox="0 0 20 20" aria-hidden="true">${
            r.dir ? DIR : DOC
          }</svg><span>${r.file}</span></p><p class="route">${r.from ? `<span class="from">${r.from}</span>` : ""}<span class="arrow" aria-label="to">→</span><span class="to">${r.to}</span></p>${
            r.note ? `<p class="note">${r.note}</p>` : ""
          }</li>`,
      )
      .join("");
    foot.textContent = scene.foot;
    plan.dataset.state = "shown";
  }

  let visible = false;
  let wake = null;
  const onScreen = () => (visible && !document.hidden ? Promise.resolve() : new Promise((r) => (wake = r)));
  const update = () => {
    if (visible && !document.hidden && wake) {
      wake();
      wake = null;
    }
  };
  new IntersectionObserver(([e]) => ((visible = e.isIntersecting), update()), { threshold: 0.3 }).observe(stage);
  document.addEventListener("visibilitychange", update);

  const caret = document.createElement("span");
  caret.className = "caret";

  async function play(scene) {
    said.textContent = "";
    plan.dataset.state = "waiting";
    desc.textContent = `Command: ${scene.parts.map((p) => p[1]).join(" ")}. ${scene.title}.`;
    for (const [kind, text] of scene.parts) {
      const part = document.createElement("span");
      part.className = kind ? `part t-${tagOf(kind)}` : "part filler";
      part.innerHTML = `<span class="w"></span><span class="k">${kind ? LABEL[kind] : ""}</span>`;
      const w = part.firstChild;
      const node = document.createTextNode("");
      w.append(node, caret);
      said.append(part);
      for (const ch of text) {
        await onScreen();
        node.data += ch;
        dispatchEvent(new CustomEvent("voxi-voice", { detail: { tag: kind ? tagOf(kind) : null } }));
        await wait(45 + Math.random() * 55);
      }
      if (kind) part.classList.add("got");
      await wait(140);
    }
    await wait(650); // Enter.
    caret.remove();
    dispatchEvent(new CustomEvent("voxi-voice", { detail: { enter: true } }));
    renderPlan(scene);
    await wait(5200);
    await onScreen();
  }

  (async () => {
    await onScreen();
    for (let n = 0; ; n++) await play(scenes[n % scenes.length]);
  })();
})();

// ---- Live demo: the real app, driven like a person would ----------------------

(() => {
  const mac = document.querySelector("[data-mac]");
  const frame = mac?.querySelector("[data-app]");
  if (!mac || !frame) return;
  const status = mac.querySelector("[data-app-status]");

  // A small Mac screen. The window is placed as the app places it
  // (show_window in app/src-tauri/src/lib.rs): 680pt wide, centred, its top
  // at 22% of the screen's height. On desktop it is shown at its real size.
  const WINDOW_WIDTH = 680;
  const SCREEN_H = 680;
  function fit() {
    const width = mac.clientWidth;
    const logicalW = Math.max(WINDOW_WIDTH + 40, Math.min(1000, width));
    mac.style.setProperty("--logical-w", logicalW);
    mac.style.setProperty("--logical-h", SCREEN_H);
    mac.style.setProperty("--scale", width / logicalW);
  }
  fit();
  new ResizeObserver(fit).observe(mac);

  // The app follows the Mac's appearance through its own
  // (prefers-color-scheme: dark) rules. Point those rules at the site's
  // switch instead, so the window shows its real light or dark styles.
  function matchTheme() {
    if (!app) return;
    const dark = document.documentElement.dataset.theme === "dark";
    for (const sheet of app.win.document.styleSheets) {
      let rules;
      try {
        rules = sheet.cssRules;
      } catch {
        continue;
      }
      for (const rule of rules) {
        if (!rule.media) continue;
        rule.__voxiMedia ??= rule.media.mediaText;
        if (!/prefers-color-scheme/.test(rule.__voxiMedia)) continue;
        const wantsDark = /prefers-color-scheme:\s*dark/.test(rule.__voxiMedia);
        rule.media.mediaText = wantsDark === dark ? "all" : "not all";
      }
    }
  }
  addEventListener("voxi-theme", matchTheme);

  function placeWindow(doc) {
    if (doc.getElementById("voxi-site-window")) return;
    const style = doc.createElement("style");
    style.id = "voxi-site-window";
    style.textContent = `html:not([data-native]) body { padding: ${Math.round(SCREEN_H * 0.22)}px 0 0; }
      .card { width: ${WINDOW_WIDTH}px; margin-inline: auto; }`;
    doc.head.append(style);
  }

  const scenes = [
    { say: "move all PDFs from Downloads to Documents", look: 3600, run: 2600 },
    { say: "clean up Downloads every 3 days", look: 4400 },
    { say: "show size of Downloads", look: 3600 },
  ];

  let token = 0;
  let app = null; // { win, box }

  // The demo is something to watch, not to use. The app focuses its own
  // command box, which would pull the visitor's typing into the frame, so
  // every real key press, paste or drop is swallowed there (the demo's own
  // typing is scripted and untrusted, so it still gets through), handed back
  // to the page, and focus leaves the frame.
  const KEY_EVENTS = ["keydown", "keyup", "keypress"];
  const INPUT_EVENTS = ["beforeinput", "paste", "cut", "drop", "dragover", "compositionstart"];
  function lock(win) {
    if (!win || win.__voxiLocked) return;
    win.__voxiLocked = true;
    const swallow = (e) => {
      if (!e.isTrusted) return;
      e.stopImmediatePropagation();
      e.preventDefault();
    };
    for (const type of INPUT_EVENTS) win.addEventListener(type, swallow, true);
    for (const type of KEY_EVENTS)
      win.addEventListener(
        type,
        (e) => {
          if (!e.isTrusted) return;
          swallow(e);
          if (type === "keypress") return;
          dispatchEvent(new KeyboardEvent(type, { key: e.key, code: e.code, bubbles: true }));
        },
        true,
      );
    const leave = () => {
      if (document.activeElement === frame) frame.blur();
    };
    win.addEventListener("focus", leave);
    win.addEventListener("focusin", leave, true);
  }

  function connect() {
    const win = frame.contentWindow;
    lock(win);
    const box = win?.document.querySelector("textarea.command");
    app = box ? { win, box } : null;
    if (app) {
      placeWindow(win.document);
      matchTheme();
    }
    return app;
  }
  const key = (target, name) =>
    target.dispatchEvent(
      new app.win.KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true }),
    );
  function setText(value) {
    const setter = Object.getOwnPropertyDescriptor(app.win.HTMLTextAreaElement.prototype, "value").set;
    setter.call(app.box, value);
    app.box.dispatchEvent(new app.win.Event("input", { bubbles: true }));
  }

  async function loop(id) {
    const alive = () => id === token;
    let n = 0;
    while (alive()) {
      const scene = scenes[n++ % scenes.length];
      if (status) status.textContent = `Typed into Voxi: ${scene.say}`;
      for (const ch of scene.say) {
        if (!alive()) return;
        setText(app.box.value + ch);
        Tray.nudge();
        await wait(38 + Math.random() * 52);
      }
      await wait(450);
      if (!alive()) return;
      key(app.box, "Enter"); // Preview
      await wait(scene.look);
      if (!alive()) return;
      if (scene.run) {
        key(app.box, "Enter"); // Enter on an empty box runs the preview.
        await wait(scene.run);
        if (!alive()) return;
      }
      key(app.win, "Escape"); // Back to an empty box.
      await wait(1400);
    }
  }

  function start() {
    if (!app && !connect()) return;
    const id = ++token;
    if (reduced.matches) {
      // No typing animation: show one finished preview and leave it.
      setText(scenes[0].say);
      key(app.box, "Enter");
      return;
    }
    loop(id);
  }
  // Escape on an empty, idle box closes the window in the real app, so only
  // reset when there is something to clear.
  function stop() {
    if (!app) return;
    token++;
    if (app.box.value || app.win.document.querySelector(".said")) key(app.win, "Escape");
  }

  let visible = false;
  let ready = false;
  const update = () => (visible && ready && !document.hidden ? start() : stop());

  async function init() {
    if (ready) return;
    // Wait for the app to render its command box.
    for (let i = 0; i < 80 && !connect(); i++) await wait(100);
    if (ready || !app) return;
    ready = true;
    update();
  }
  frame.addEventListener("load", init);
  if (frame.contentDocument?.readyState === "complete") init();
  new IntersectionObserver(
    ([entry]) => {
      if (entry.isIntersecting === visible) return;
      visible = entry.isIntersecting;
      update();
    },
    { threshold: 0.35 },
  ).observe(mac);
  document.addEventListener("visibilitychange", update);
})();

// ---- Search the command reference ---------------------------------------------

(() => {
  const search = document.querySelector("[data-command-search]");
  if (!search) return;
  const sections = [...document.querySelectorAll(".catalog section")];
  const status = document.querySelector("[data-search-status]");
  const empty = document.querySelector("[data-search-empty]");
  const links = [...document.querySelectorAll(".toc a")];
  const total = sections.reduce((count, s) => count + s.querySelectorAll(".say li").length, 0);
  function filter() {
    const words = search.value.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
    let visible = 0;
    sections.forEach((section) => {
      const heading = section.querySelector("h2").textContent.toLocaleLowerCase();
      let matches = 0;
      section.querySelectorAll(".say li").forEach((row) => {
        const text = heading + " " + row.textContent.toLocaleLowerCase();
        row.hidden = !words.every((word) => text.includes(word));
        if (!row.hidden) matches++;
      });
      section.hidden = matches === 0;
      visible += matches;
      const link = links.find((item) => item.hash === "#" + section.id);
      if (link) link.parentElement.hidden = matches === 0;
    });
    empty.hidden = visible > 0;
    status.textContent = words.length
      ? `${visible} of ${total} examples match.`
      : `${total} examples. Search by action, file type, or phrase.`;
  }
  search.addEventListener("input", filter);
  filter();
  const observer = new IntersectionObserver(
    (entries) => {
      const entry = entries.find((item) => item.isIntersecting);
      if (!entry) return;
      links.forEach((link) => {
        if (link.hash === "#" + entry.target.id) link.setAttribute("aria-current", "location");
        else link.removeAttribute("aria-current");
      });
    },
    { rootMargin: "-120px 0px -60% 0px" },
  );
  sections.forEach((section) => observer.observe(section));
})();

// ---- The wave field --------------------------------------------------------------
// Sound waves across the whole page, behind everything, like the smile in the
// icon. They swell around the pointer, a click sends a ripple through them, and
// they speak with the hero sentence: each typed letter is a small pulse in the
// colour of the part being typed, and Enter sends a ripple from the stage.
// Still with reduced motion; paused when the tab is hidden.

(() => {
  const canvas = document.createElement("canvas");
  canvas.className = "field";
  canvas.setAttribute("aria-hidden", "true");
  document.body.prepend(canvas);
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  const STRANDS = 11;
  const KEYS = ["t-to", "t-from", "t-what", "t-do", "t-when"];
  let w = 0, h = 0, dpr = 1;
  let colors = {}, lineAlpha = 0.2;
  const pointer = { x: -1e4, y: -1e4, tx: -1e4, ty: -1e4, on: 0, ton: 0 };
  let voice = 0; // Swells with each typed letter, then fades.
  let voiceColor = null;
  const ripples = []; // { x, y, t0, color }
  let running = false, last = 0, t = 0;

  function readColors() {
    const cs = getComputedStyle(document.documentElement);
    for (const k of KEYS) colors[k] = cs.getPropertyValue(`--${k}`).trim() || "#2a6ad9";
    lineAlpha = document.documentElement.dataset.theme === "dark" ? 0.26 : 0.2;
  }
  function size() {
    dpr = Math.min(devicePixelRatio || 1, 2);
    w = innerWidth;
    h = innerHeight;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }

  function draw() {
    ctx.clearRect(0, 0, w, h);
    const px = pointer.x, py = pointer.y, on = pointer.on;
    const sx = Math.max(160, w * 0.14), sy = Math.max(120, h * 0.16);
    const scroll = scrollY * 0.15;
    const now = t;
    const step = w > 1600 ? 7 : 6;

    // A soft light that follows the pointer.
    if (on > 0.01) {
      const g = ctx.createRadialGradient(px, py, 0, px, py, Math.max(w, h) * 0.35);
      g.addColorStop(0, withAlpha(voiceColor || colors["t-to"], 0.1 * on));
      g.addColorStop(1, withAlpha(colors["t-to"], 0));
      ctx.fillStyle = g;
      ctx.fillRect(0, 0, w, h);
    }

    ctx.lineWidth = 1.4;
    ctx.lineCap = "round";
    for (let i = 0; i < STRANDS; i++) {
      const base = ((i + 0.5) / STRANDS) * h + Math.sin(i * 1.7 + scroll * 0.01) * 14;
      const k1 = 0.0026 + (i % 4) * 0.0007;
      const k2 = 0.0061 + (i % 3) * 0.0011;
      const amp = 10 + (i % 5) * 4;
      const phase = i * 1.31 + scroll * 0.02;
      const near = Math.exp(-((base - py) ** 2) / (2 * sy * sy)) * on;
      const color = colors[KEYS[i % KEYS.length]];
      ctx.strokeStyle = withAlpha(color, lineAlpha * (0.55 + near * 0.9 + voice * 0.35));
      ctx.beginPath();
      for (let x = -step; x <= w + step; x += step) {
        let y =
          base +
          Math.sin(x * k1 + now * 0.35 + phase) * amp +
          Math.sin(x * k2 - now * 0.22 + phase * 2) * amp * 0.45;
        // The pointer speaks: a fast ripple, enveloped like the icon's smile.
        const gx = Math.exp(-((x - px) ** 2) / (2 * sx * sx));
        y += gx * near * 26 * Math.sin(x * 0.045 - now * 5 + i);
        // Typing: the whole field hums a little.
        y += voice * 9 * Math.sin(x * 0.03 - now * 7 + i * 0.8) * Math.sin((x / w) * Math.PI);
        // Ripples from clicks and from Enter.
        for (const r of ripples) {
          const age = now - r.t0;
          const radius = age * 520;
          const d = Math.hypot(x - r.x, base - r.y) - radius;
          if (d > -90 && d < 90) {
            const fade = Math.max(0, 1 - age / 2.4);
            y += Math.cos((d / 90) * Math.PI * 0.5) ** 2 * 22 * fade * Math.sin(d * 0.08);
          }
        }
        if (x === -step) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
    }
  }

  function withAlpha(color, a) {
    a = Math.max(0, Math.min(1, a));
    if (color.startsWith("#") && color.length === 7) {
      const n = parseInt(color.slice(1), 16);
      return `rgba(${n >> 16},${(n >> 8) & 255},${n & 255},${a})`;
    }
    return color;
  }

  function frame(ms) {
    if (!running) return;
    const dt = Math.min(0.05, (ms - (last || ms)) / 1000);
    last = ms;
    t += dt;
    const ease = 1 - Math.exp(-dt * 6);
    if (pointer.x < -1e3) (pointer.x = pointer.tx), (pointer.y = pointer.ty);
    pointer.x += (pointer.tx - pointer.x) * ease;
    pointer.y += (pointer.ty - pointer.y) * ease;
    pointer.on += (pointer.ton - pointer.on) * ease;
    voice *= Math.exp(-dt * 2.2);
    for (let i = ripples.length - 1; i >= 0; i--) if (t - ripples[i].t0 > 2.4) ripples.splice(i, 1);
    draw();
    requestAnimationFrame(frame);
  }
  function start() {
    if (running || reduced.matches || document.hidden) return;
    running = true;
    last = 0;
    requestAnimationFrame(frame);
  }
  function stop() {
    running = false;
  }
  function refresh() {
    if (reduced.matches) {
      stop();
      pointer.on = 0;
      voice = 0;
      ripples.length = 0;
      draw(); // One still frame.
    } else start();
  }

  addEventListener(
    "pointermove",
    (e) => {
      if (e.pointerType === "touch") return;
      pointer.tx = e.clientX;
      pointer.ty = e.clientY;
      pointer.ton = 1;
    },
    { passive: true },
  );
  document.documentElement.addEventListener("pointerleave", () => (pointer.ton = 0));
  addEventListener("pointerdown", (e) => {
    if (reduced.matches) return;
    ripples.push({ x: e.clientX, y: e.clientY, t0: t });
    if (ripples.length > 6) ripples.shift();
  });
  addEventListener("voxi-voice", (e) => {
    if (reduced.matches) return;
    const { tag, enter, at } = e.detail || {};
    if (enter && at) {
      ripples.push({ x: at.x, y: at.y, t0: t });
      return;
    }
    if (enter) {
      const stage = document.querySelector("[data-stage]")?.getBoundingClientRect();
      if (stage) ripples.push({ x: stage.left + stage.width / 2, y: stage.top + stage.height / 2, t0: t });
      return;
    }
    voice = Math.min(1, voice + 0.12);
    voiceColor = tag ? colors[`t-${tag}`] : null;
  });
  addEventListener("voxi-theme", () => (readColors(), reduced.matches && draw()));
  addEventListener("resize", () => (size(), reduced.matches && draw()));
  document.addEventListener("visibilitychange", () => (document.hidden ? stop() : start()));
  reduced.addEventListener?.("change", refresh);

  readColors();
  size();
  refresh();
})();

// ---- The closing keys -----------------------------------------------------------
// ⌃, ⌥ and Space on the page press down when the visitor presses them on their
// keyboard, or clicks them. All three together is Voxi's shortcut: the logo
// perks up and a ripple goes through the wave field.

(() => {
  const keys = [...document.querySelectorAll(".keys [data-key]")];
  if (!keys.length) return;
  const logo = document.querySelector(".closing .section-logo");
  const held = new Set();
  // Match the physical key, not the character: Option+Space types a
  // non-breaking space on a Mac, so `key` alone would miss it.
  const which = (e) =>
    /^Control/.test(e.code) || e.key === "Control"
      ? "Control"
      : /^Alt/.test(e.code) || e.key === "Alt"
        ? "Alt"
        : e.code === "Space" || e.key === " " || e.key === "\u00a0"
          ? " "
          : null;

  // Each key has its own state. A quick tap still shows for a moment.
  const MIN_DOWN = 160;
  const since = new Map(); // key -> when it went down
  const timers = new Map();
  const el = (name) => keys.find((k) => k.dataset.key === name);
  function press(name) {
    clearTimeout(timers.get(name));
    if (!held.has(name)) since.set(name, performance.now());
    held.add(name);
    el(name)?.classList.add("down");
    if (keys.every((k) => held.has(k.dataset.key))) heard();
  }
  function release(name) {
    if (!held.delete(name)) return;
    const left = MIN_DOWN - (performance.now() - (since.get(name) || 0));
    const lift = () => !held.has(name) && el(name)?.classList.remove("down");
    if (left > 0) timers.set(name, setTimeout(lift, left));
    else lift();
  }
  let heardTimer = 0;
  // The easter egg: pressing Voxi's shortcut on the website, where there is
  // no Voxi to open yet.
  const egg = document.querySelector("[data-egg]");
  let eggTimer = 0;
  function speak() {
    if (!egg) return;
    egg.textContent = "Ooh, almost! Download me and try again.";
    egg.classList.add("shown");
    clearTimeout(eggTimer);
    eggTimer = setTimeout(() => egg.classList.remove("shown"), 3200);
  }
  function heard() {
    speak();
    document.querySelector(".keys .try")?.classList.add("done"); // They found it.
    if (!logo) return;
    logo.classList.add("heard");
    clearTimeout(heardTimer);
    heardTimer = setTimeout(() => logo.classList.remove("heard"), 450);
    const box = logo.getBoundingClientRect();
    dispatchEvent(
      new CustomEvent("voxi-voice", { detail: { enter: true, at: { x: box.left + box.width / 2, y: box.top + box.height / 2 } } }),
    );
  }

  addEventListener("keydown", (e) => {
    const name = which(e);
    if (!name || e.repeat || e.target.closest?.("input, textarea, [contenteditable]")) return;
    press(name);
  });
  addEventListener("keyup", (e) => {
    const name = which(e);
    if (name) release(name);
    // While ⌘ is held macOS sends no keyup for Space, so lift it with ⌘.
    if (e.key === "Meta") release(" ");
  });
  // A key released while the window was in the background never sends keyup.
  // The demo frame taking focus also blurs this window, but the page still has
  // focus then, so only let go when focus has really left the page.
  addEventListener("blur", () =>
    setTimeout(() => document.hasFocus() || [...held].forEach(release)),
  );

  // Clicking presses a key for as long as the pointer is down. With ⌃ held a
  // click is a right-click on a Mac: its menu would swallow the release, so
  // the menu is suppressed and the pointer captured, and any way the press
  // can end lets go of the key.
  keys.forEach((k) => {
    const name = k.dataset.key;
    let pointer = null;
    const up = () => {
      if (pointer === null) return;
      pointer = null;
      release(name);
    };
    k.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      pointer = e.pointerId;
      try {
        k.setPointerCapture(e.pointerId);
      } catch {}
      press(name);
    });
    k.addEventListener("contextmenu", (e) => e.preventDefault());
    for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) k.addEventListener(type, up);
    addEventListener("pointerup", up);
  });

  // A modifier's keyup can go missing (a menu, another app, the shortcut
  // itself). Every key and pointer event says which modifiers are really
  // down, so trust that over what was last seen.
  const MODS = { Control: "ctrlKey", Alt: "altKey" };
  const reconcile = (e) => {
    for (const [name, flag] of Object.entries(MODS))
      if (held.has(name) && !e[flag] && !(e.type === "keydown" && which(e) === name)) release(name);
  };
  for (const type of ["keydown", "keyup", "pointerdown", "pointermove", "pointerup"])
    addEventListener(type, reconcile, true);
})();
