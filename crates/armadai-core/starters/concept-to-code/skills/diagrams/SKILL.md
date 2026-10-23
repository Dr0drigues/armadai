---
name: diagrams
description: "When and how to draw a diagram for the user to validate: Mermaid in the technical documentation (docs/technical/...) plus a standalone HTML rendering. Use when designing an architecture, components, a sequence between parts, a data model, a lifecycle or a deployment, and whenever a structure must be validated visually."
---
# Diagrams

A structure the user has to approve is approved from a picture, not from paragraphs. Draw one whenever it helps a decision; skip it when it would only restate a sentence.

## When

- An architecture with more than two components, or one that changes.
- An interaction across parts: a request through front end, API and database; a background job; an external service.
- A data model with relations.
- A lifecycle with states (an order, a task, a session).
- A deployment: what runs where, what talks to what.
- Any time the user must choose between two designs: one diagram per option.

## Which kind (Mermaid)

| To show | Mermaid |
|---|---|
| components and dependencies | `flowchart LR` |
| interactions in time | `sequenceDiagram` |
| data and relations | `erDiagram` |
| states and transitions | `stateDiagram-v2` |
| deployment | `flowchart` with one `subgraph` per machine or service |

Keep each diagram readable: about fifteen nodes at most, split otherwise. Label every arrow with what goes through it (a request, an event, data). Use the names of the brief and of the code.

## Where

- In the human documentation, `docs/technical/<area>/<subject>.md`, `<area>` being `architecture`, `api`, `infra` or `data`: a ```` ```mermaid ```` block, followed by a few sentences that explain it.
- Beside it, `docs/technical/<area>/<subject>.html`, which renders the same Mermaid text so the user opens it in a browser. Keep the two texts identical: copy the block, do not redraw it.

```html
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>architecture: workspace</title></head>
<body>
<h1>architecture: workspace</h1>
<pre class="mermaid">
flowchart LR
  ui[Web app] -->|HTTP JSON| api[API]
  api -->|SQL| db[(Database)]
</pre>
<script type="module">
  import mermaid from 'https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs';
  mermaid.initialize({ startOnLoad: true });
</script>
</body></html>
```

The rendering loads Mermaid from a pinned CDN version, so it needs a network connection when opened; the Markdown file stays the source of truth.

## Getting it validated

After writing an `.html` file, offer the user `/preview <file>` to open it (in the main conversation; a subagent returns the path to `lead`, who offers it). Return, or give the user, the path of the HTML file, what to look at ("the API never reaches the database of the billing service"), and the question to settle, with options when there are several designs. A subagent cannot ask the user: it returns the question to `lead`, who asks. Record the answer in the document, in a dated "Decisions" section.

Write the explanations in the user's language; identifiers stay as in the code.
