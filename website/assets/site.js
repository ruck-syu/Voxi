// Points the download buttons at the newest DMG, and plays the hero demo.

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
    say: "Run my usuals.",
    dictated: true,
    title: "Run my usuals: 2 commands",
    rows: [
      ["Screenshot 2026-10-28 at 09.14.png", "→ ~/Desktop/Screenshots"],
      ["Screenshot 2026-10-28 at 11.40.png", "→ ~/Desktop/Screenshots"],
      ["Notion-Setup.dmg", "→ Trash", true],
    ],
    more: null,
    done: "Done. 3 changes.",
  },
];

const demo = document.querySelector("[data-demo]");
if (demo) runDemo(demo);

function runDemo(root) {
  const command = root.querySelector(".demo-command");
  const body = root.querySelector(".demo-body");
  const foot = root.querySelector(".demo-foot span");
  const mic = root.querySelector(".dictating");
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
      if (scene.dictated) {
        // Dictation lands in bursts of words, the way Wispr Flow types.
        mic.setAttribute("data-on", "");
        await wait(1300);
        const words = scene.say.split(" ");
        for (let i = 0; i < words.length; i += 3) {
          command.textContent = words.slice(0, i + 3).join(" ");
          await wait(160);
        }
        mic.removeAttribute("data-on");
      } else {
        for (let i = 1; i <= scene.say.length; i++) {
          command.textContent = scene.say.slice(0, i);
          await wait(34 + Math.random() * 40);
        }
      }
      await wait(600);
      command.textContent = "";
      preview(scene);
      await wait(3400);
      done(scene);
      await wait(2200);
    }
  })();
}
