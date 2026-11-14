// Points the download buttons at the newest DMG, switches light and dark
// mode, and plays the hero demo.

const REPO = "ruck-syu/Voxi";

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
    if (version) labels.forEach((el) => (el.textContent = `Version ${version} for macOS 12 or later.`));
  } catch {
    // Offline or rate-limited: keep the releases-page link.
  }
})();

// ---- Light and dark mode -------------------------------------------------------

// Follows the Mac's appearance until someone picks a mode with the switch;
// that choice is remembered in this browser. The <head> applies it before paint.
(() => {
  const root = document.documentElement;
  const system = matchMedia("(prefers-color-scheme: dark)");
  const current = () => root.dataset.theme || (system.matches ? "dark" : "light");
  const switches = document.querySelectorAll("[data-theme-switch]");

  const label = () => {
    const next = current() === "dark" ? "light" : "dark";
    switches.forEach((b) => b.setAttribute("aria-label", `Switch to ${next} mode`));
  };

  switches.forEach((button) =>
    button.addEventListener("click", () => {
      const next = current() === "dark" ? "light" : "dark";
      root.dataset.theme = next;
      document.querySelectorAll('meta[name="theme-color"]').forEach((m) => {
        m.removeAttribute("media");
        m.content = next === "dark" ? "#121a2e" : "#ffffff";
      });
      try {
        localStorage.setItem("voxi-theme", next);
      } catch {
        // Storage blocked: the choice lasts for this page only.
      }
      label();
    }),
  );
  system.addEventListener("change", label);
  label();
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

    window.setTimeout(() => {
      button.textContent = "Copy";
    }, 1800);
  });
});

// ---- Hero face: the icon's smile is a sound wave that ripples while it listens --

// Same curve as docs/icon/appicon.swift, in the icon's 100-unit box: a downward
// arc with three ripples riding on it. `level` 0 is the icon at rest.
function smilePath(level, phase) {
  const n = 64;
  const ripple = 2.6 + 3.4 * level;
  let d = "";
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    const s = Math.sin(Math.PI * t);
    const x = 26 + 48 * t;
    const y = 62 + 8 * s + ripple * Math.sin(6 * Math.PI * t - phase) * s;
    d += `${i ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`;
  }
  return d;
}

function makeFace(svg) {
  const smile = svg.querySelector(".smile");
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  let level = 0;
  let target = 0;
  let phase = 0;
  let frame = 0;
  smile.setAttribute("d", smilePath(0, 0));

  const tick = () => {
    level += (target - level) * 0.08;
    // While listening the wave travels; at rest it eases back to the icon's phase.
    const rest = Math.round(phase / (2 * Math.PI)) * 2 * Math.PI;
    if (target > 0) phase += 0.09 + 0.05 * level;
    else phase += (rest - phase) * 0.08;
    smile.setAttribute("d", smilePath(level, phase));
    const settled = target === 0 && level < 0.002 && Math.abs(rest - phase) < 0.002;
    frame = settled ? 0 : requestAnimationFrame(tick);
    if (settled) smile.setAttribute("d", smilePath(0, 0));
  };

  return {
    listen(on) {
      if (reduced) return;
      target = on ? 1 : 0;
      if (!frame) frame = requestAnimationFrame(tick);
    },
    blink() {
      if (reduced) return;
      svg.removeAttribute("data-blink");
      void svg.getBoundingClientRect();
      svg.setAttribute("data-blink", "");
    },
  };
}

// ---- Hero demo: a replica of Voxi's window --------------------------------------

const SCENES = [
  {
    say: "move all PDFs from Downloads to Documents",
    dictated: false,
    title: "Move 26 files to Documents",
    rows: [
      ["Q3 board deck.pdf", "→ ~/Documents"],
      ["lease-renewal-2026.pdf", "→ ~/Documents"],
      ["report.pdf", "→ ~/Documents/report 2.pdf"],
      ["tax-receipt-march.pdf", "→ ~/Documents"],
    ],
    more: "and 22 more steps",
    done: "Done. 26 changes.",
  },
  {
    say: "Clean up Downloads older than 90 days.",
    dictated: true,
    title: "Move 14 files from Downloads to the Trash",
    rows: [
      ["Zoom.pkg", "→ Trash", true],
      ["Figma-124.3.dmg", "→ Trash", true],
      ["old-export.csv", "→ Trash", true],
      ["IMG_2231 copy.heic", "→ Trash", true],
    ],
    more: "and 10 more steps",
    done: "Done. 14 changes.",
  },
  {
    say: "Delete dmg files from Downloads every 3 days.",
    dictated: true,
    title: "Repeat every 3 days: move 3 files to the Trash",
    rows: [
      ["Notion-Setup.dmg", "→ Trash", true],
      ["Arc-1.62.dmg", "→ Trash", true],
      ["Hex.dmg", "→ Trash", true],
    ],
    more: null,
    done: "Scheduled. Repeats every 3 days.",
  },
];

const demo = document.querySelector("[data-demo]");
const faceSvg = document.querySelector("[data-face]");
if (demo) runDemo(demo, faceSvg ? makeFace(faceSvg) : { listen() {}, blink() {} });

function runDemo(root, face) {
  const command = root.querySelector(".demo-command");
  const body = root.querySelector(".demo-body");
  const foot = root.querySelector(".demo-foot span");
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const tidy = (s) => s.replace(/[.!?]+$/, "");

  const preview = (scene) => {
    const rows = scene.rows
      .map(([name, to, trash], i) => `<li style="--i:${i}"${trash ? " data-trash" : ""}><b>${esc(name)}</b><span>${esc(to)}</span></li>`)
      .join("");
    const more = scene.more ? `<li style="--i:${scene.rows.length}"><span>${esc(scene.more)}</span></li>` : "";
    body.innerHTML = `<p class="demo-said">“${esc(tidy(scene.say))}”</p><p class="demo-title">${esc(scene.title)}</p><ul class="demo-rows">${rows}${more}</ul>`;
    foot.textContent = "Enter to run. Nothing changes until then.";
  };

  const done = (scene) => {
    body.innerHTML = `<p class="demo-said">“${esc(tidy(scene.say))}”</p><p class="demo-title"><svg class="tick" viewBox="0 0 20 20" aria-hidden="true"><path d="M4 10.5l4 4 8-9"/></svg>${esc(scene.done)}</p>`;
    foot.textContent = "Say the next command.";
  };

  if (reduced) {
    command.textContent = "";
    preview(SCENES[0]);
    return;
  }

  (async () => {
    for (let n = 0; ; n = (n + 1) % SCENES.length) {
      const scene = SCENES[n];
      body.innerHTML = "";
      command.textContent = "";
      foot.textContent = "Enter to preview.";
      await wait(900);
      face.listen(true);
      if (scene.dictated) {
        // Dictation lands in bursts of words, the way dictation apps paste.
        await wait(500);
        const words = scene.say.split(" ");
        for (let i = 0; i < words.length; i += 3) {
          command.textContent = words.slice(0, i + 3).join(" ");
          await wait(160);
        }
      } else {
        for (let i = 1; i <= scene.say.length; i++) {
          command.textContent = scene.say.slice(0, i);
          await wait(34 + Math.random() * 40);
        }
      }
      face.listen(false);
      await wait(600);
      command.textContent = "";
      preview(scene);
      await wait(3400);
      done(scene);
      face.blink();
      await wait(2200);
    }
  })();
}
