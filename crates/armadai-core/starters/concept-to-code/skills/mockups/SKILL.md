---
name: mockups
description: "Mockups as one standalone HTML file with three variants in tabs, the user's choice recorded, then new rounds from the chosen variant. Use when a screen, page, layout or interface must be proposed (/mockup), and on any feedback about an existing mockup (/iterate), including plain-language feedback such as \"make B lighter\", \"mix A and C\", \"I prefer the header of A\", \"refais la B plus aérée\"."
---
# Mockups

The user chooses what they will get by seeing it, not by reading about it. Each round is one HTML file with three variants; the user picks one, or a mix; the choice and its reason are recorded; the next round starts from it.

## Files

- `mockups/<subject>/v<n>.html`: `<subject>` in kebab-case (`project-board`, `login`), `n` the round, starting at 1. A new round is always a new file: never overwrite an earlier one.
- `.armadai/docs/design-decisions.md`: the choices, one entry per round.

Before drawing, read the brief (`.armadai/docs/brief.md`), the frame (`.armadai/docs/cadre.md`), the earlier entries of `design-decisions.md` for this subject, the skills of the `ux-guidelines` slot, and the product's existing styles if it has any (reuse its colours, fonts, spacing).

## The HTML file

- One file, no dependency: CSS and JavaScript inline, no external URL (no CDN, web font, remote image). It opens offline with a double click. Draw images and icons with CSS or inline SVG; use system fonts.
- Three tabs at the top, **A**, **B**, **C**, showing one variant at a time; a tab is a `<button>` (keyboard reachable), the active one is visible.
- Each variant is a real alternative (layout, density, navigation, the order of information), not the same screen in three colours.
- Above each variant, one line: its name, its idea, its trade-off.
- Real content from the brief (the user's projects, tasks, numbers), never lorem ipsum.
- It holds at a desktop width and at a phone width when the product may be used on one.
- A comment at the top: subject, round, date, and what it starts from ("from v1 B, with the header of A; feedback: lighter").

Skeleton:

```html
<!doctype html>
<html lang="<user's language>"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>project-board v1</title>
<!-- subject: project-board, round 1, YYYY-MM-DD, from: the brief -->
<style>
  body { margin: 0; font-family: system-ui, sans-serif; }
  .tabs { display: flex; gap: 8px; padding: 12px; border-bottom: 1px solid #ddd; }
  .tabs button[aria-selected="true"] { font-weight: 700; border-bottom: 3px solid currentColor; }
  .variant { display: none; padding: 16px; }
  .variant.active { display: block; }
  .caption { color: #555; margin: 0 0 16px; }
</style></head>
<body>
<nav class="tabs" role="tablist">
  <button role="tab" aria-selected="true" data-v="a">A</button>
  <button role="tab" aria-selected="false" data-v="b">B</button>
  <button role="tab" aria-selected="false" data-v="c">C</button>
</nav>
<section class="variant active" id="a"><p class="caption">A, Kanban: …</p><!-- the screen --></section>
<section class="variant" id="b"><p class="caption">B, List: …</p></section>
<section class="variant" id="c"><p class="caption">C, Timeline: …</p></section>
<script>
  document.querySelectorAll('.tabs button').forEach(b => b.onclick = () => {
    document.querySelectorAll('.tabs button').forEach(x => x.setAttribute('aria-selected', x === b));
    document.querySelectorAll('.variant').forEach(s => s.classList.toggle('active', s.id === b.dataset.v));
  });
</script>
</body></html>
```

## The choice

Give the path of the file to open and one line per variant, then have the user choose:

- In the main conversation (`/mockup`, `/iterate`), ask with your question tool: A, B, C, and a mix; several answers are allowed ("the header of A with the list of C"). Ask also, in one short question, why: the reason guides the next round.
- As a subagent (`designer`), you cannot ask: return the file, the variants and the question to `lead` under "Questions for the user"; `lead` asks and sends the answer back.

Record the choice in `.armadai/docs/design-decisions.md`:

```markdown
## <subject>, v<n>, YYYY-MM-DD
Chosen: B, with the header of A
Why: <the user's reason, in their words>
Set aside: A (<why>), C (<why>)
Still open: <what the next round must settle, if anything>
```

## Iterating

A round starts from the last choice recorded for the subject, plus the user's feedback, whether typed after `/iterate` or said in plain words ("make B lighter", "mix A and C", "the cards are too big"):

1. Find the subject and the variant the feedback is about. If you cannot tell which, ask.
2. Write `v<n+1>.html`: three new variants, all built on the chosen one, each answering the feedback a different way or to a different degree (lighter by spacing, by fewer borders, by fewer elements shown).
3. Have the user choose again and record the entry, as above.

Stop iterating when the user says the mockup is good: record it as final ("Chosen: v3 B, final"). Speak the user's language in the captions, the questions and the record.
