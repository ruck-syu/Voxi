# Voxi website redesign

Source brief: [Anurag Jha’s post](https://x.com/anuragdotdev/status/2121490763175678346), inspected November 15, 2026. The post contains seven resources; two visible replies add Refero Styles and Bencho. All nine public sites were inspected. No account-gated source or paid component code was copied.

## Design direction

Voxi is a small, local Mac file utility. The page should make the sentence → preview → confirmation workflow tangible, while keeping its familiar face, navy eyes, and coral smile.

Palette: cool paper `#f7f9fd`, white surface `#ffffff`, navy ink `#1b2a4a`, slate text `#59677f`, desktop blue `#dbe7f7`, coral `#c64037`. A matching dark palette follows system appearance or the saved theme choice.

Typography: self-hosted Geist for display, headings and reading, system monospace for the Homebrew command. The desktop app uses Geist too. Font license files ship with the fonts.

Layout: centered introductory copy above a broad desktop demonstration; left-aligned content and purposeful file-operation illustrations below. The command reference uses a category sidebar on desktop, wrapping links on mobile. Installation is a numbered sequence beside a download and Homebrew panel.

```text
Home                         Commands                  Install
brand / navigation           brand / navigation        brand / navigation
centered headline            categories | intro        intro
command preview                        | search       steps | download
example selector                       | examples           | Homebrew
preview / undo / local                                 first command
commands / routines
personalization / download
```

Review against the brief: a generic animated component showcase would bury Voxi’s purpose. Keep the app preview as the memorable element, use interaction to explain real actions, and use the reference libraries to shape useful components. Avoid continuous text rotation, decorative particle fields, and autoplay demos. Preserve the original 49 reference examples and all installation instructions.

## Resource-to-component mapping

These are original HTML/CSS/JavaScript adaptations of observed patterns, not literal React package imports. The existing site remains buildless and adds no runtime dependencies.

| Resource                                                                 | Patterns inspected                                                                | Application                                                                       |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| [21st](https://21st.dev/community/components)                            | Product-led hero, liquid-glass button, category chips, searchable catalogue       | Broad app-window hero, translucent sticky navigation, example buttons and search  |
| [Designeer](https://designeer.xyz/)                                      | Fixed browsing sidebar, compact filters, generous gallery spacing                 | Command reference sidebar and category links                                      |
| [React Bits Stack](https://reactbits.dev/components/stack)               | Layered card stack, explicit interaction, optional autoplay                       | Layered document illustration; user-selected preview scenes rather than autoplay  |
| [Magic UI Bento Grid](https://magicui.design/docs/components/bento-grid) | File and calendar feature panels                                                  | Preview, undo, and local-processing panels; routine calendar                      |
| [Magic UI Terminal](https://magicui.design/docs/components/terminal)     | Compact terminal presentation                                                     | Copyable Homebrew block                                                           |
| [Uiverse](https://uiverse.io/)                                           | CSS buttons, switches, card surfaces, input fields                                | Tactile download/run buttons, appearance switch, search input, copy feedback      |
| [Mobbin](https://mobbin.com/)                                            | Strong product typography, restrained navigation, clear primary/secondary actions | Page hierarchy, download priority, readable installation sequence                 |
| [Inspora](https://www.inspora.design/)                                   | Category-based navigation and sparse gallery framing                              | Category navigation, quiet framing around the product demonstration               |
| [Refero Styles](https://styles.refero.design/)                           | Explicit color/type/spacing/component systems                                     | Shared semantic tokens, typography and light/dark surfaces across all three pages |
| [Bencho](https://bencho.dev/)                                            | Card stack, checklist, action-driven micro-interactions                           | Select → preview → run → reset demo and clipboard status feedback                 |

21st’s public catalogue is accessible but component source prompts require an account. Mobbin, Inspora, and Refero are treated as visual references; their gallery content is not reused. Designeer displays All Rights Reserved. Bencho includes paid source; none is copied. React Bits identifies its free library as MIT + Commons Clause; no source is vendored. Uiverse identifies its UI elements as MIT. The website contains authored implementations and self-hosted fonts with their own included licenses.

## Behavior and accessibility

- The home demo has three selectable scenes and a run/reset action. It never accesses files or claims to run an actual Mac command.
- Search filters the reference by all entered words, exposes result counts and an empty state, and hides categories without matches.
- Existing dynamic release links, persistent appearance selection, and Homebrew copy feedback remain.
- Skip links, visible keyboard focus, native buttons/search input, pressed/current states, live result feedback, reduced-motion behavior and forced-color boundaries are included.
- Local links retain the existing Vercel clean-URL configuration. No hosting settings are changed.

## Validation

- Chrome: homepage, commands and installation at 320, 390, 768, 1024 and 1440 CSS pixels, in both light and dark appearance; no horizontal overflow.
- Demo: all scene choices, run, schedule and reset; search: matching, no matches, and clearing back to all 49 examples.
- Persistent appearance choice and actual Homebrew clipboard contents; release API failure retains the releases-page fallback.
- Original command example text preserved; unique IDs, local links and asset paths checked. Reference remains readable with JavaScript disabled.
- JavaScript syntax check and Git whitespace check pass; no page JavaScript errors during browser checks.
- Desktop and mobile visual review used local headless Chrome after T3 preview screenshots timed out and the paired browser extension was unavailable.

## Product imagery revision

The initial illustrative window, file stack, calendar, and feature drawings have been replaced with captures of Voxi’s actual React frontend. Screenshots show the frosted card, coral head rule, ruled result rows, preview actions, completion state, repeating-task warning, and Settings. They were captured from the existing development preview at 680 CSS pixels of app content, with its existing `app/src/mock.ts` sample backend. These are actual frontend renders with sample files, not native macOS window captures; native vibrancy depends on the real desktop behind the window. No app source or personal files were changed to produce them.

`website/assets/product/` contains light/dark WebP captures, a short H.264 walkthrough recorded while typing, previewing, and confirming a PDF move, and the actual bundled app icon optimized as WebP. The walkthrough is about 235 KB; screenshots range from roughly 36–76 KB each. Capture timestamps in the scheduling screen come from the existing preview backend.

The homepage now uses the recording with visible playback controls, screenshot selectors with animated changes, a short entrance sequence, button feedback, and native scroll-linked motion on the scheduling screenshot and closing icon. Scroll motion is feature-detected with a static fallback; reduced-motion disables spatial animation and automatic playback. Playback pauses when offscreen or when the document is hidden. Sample-file captions and a full-size screenshot link make the imagery understandable on phones.

Revision verification: all three pages fit 320, 390, 768, and 1440px in light and dark mode. Screenshot selection, matching full-size links, appearance switching, manual playback/pause, reduced-motion autoplay prevention, and pausing offscreen were exercised in Chrome. Search still restores all 49 examples. No broken images, asset HTTP failures, or page JavaScript errors occurred. The existing phone tunnel served the revised homepage, WebP screenshot, and MP4 with HTTP 200.

## Walkthrough selection correction

The screenshot selector previously changed only the still image; Play always resumed the PDF-move recording. Each scene now selects its matching source, poster, and accessible video description. Switching scenes pauses and resets the old recording before playing the new one. Scheduling has a dedicated capture showing the repeat command, warning, Schedule repeating task action, and repeating-task confirmation. Cleanup has a separate preview-and-cancel recording. Only the selected clip is loaded; native lazy loading is paired with preload none for other browsers.

Verified all three selections on a 390px phone viewport, including rapid changes between scenes: each plays its matching MP4 with no page JavaScript errors.

## Blue glass backdrop revision

All three pages now share a fixed, layered blue atmosphere with frosted navigation, translucent feature and installation panels, glass edges, and subtle inset highlights. Light appearance uses blue paper with navy text; dark appearance uses deep blue glass and pale text. Product screenshots and recordings are unchanged. New gradients explicitly interpolate in OKLab. Backdrop filtering is feature-detected, with opaque panel fallbacks and Safari-prefixed support. Reduced-transparency preferences receive opaque surfaces without blur; forced colors remove the decorative atmosphere.

Verified all three pages at 320, 390, 768, and 1440px in light and dark modes, plus manual theme switching and reduced-transparency emulation. No horizontal overflow or page JavaScript errors. Darkened secondary text for contrast over the blue background.
