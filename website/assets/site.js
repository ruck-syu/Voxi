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
        await wait(45 + Math.random() * 55);
      }
      if (kind) part.classList.add("got");
      await wait(140);
    }
    await wait(650); // Enter.
    caret.remove();
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

  function connect() {
    const win = frame.contentWindow;
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
