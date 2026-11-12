// The ArmadAI mod, written as is by `armadai link` for `ui.panel: true` and `ui.flow_band: true`.
// Do not edit it: change project.yaml and run `armadai link` again.
// It runs `armadai flow status --json` every 2 seconds. The panel lists the runs and the
// Markdown files of the folders in panel.json (`ui.documents`) changed since the session
// began; the band above the prompt (`"band": true` in panel.json) shows the open run on
// one line. `"pane": false` in panel.json: no panel, the band alone.
const PANE = 'armadai-flows'
const SCHEMA = 'armadai.flow-status/1'
const EVERY_MS = 2000
const MAX_DOCUMENTS = 20
const MAX_DEPTH = 4

// What the pane draws: set by `refresh`, read by `ui.render`.
let view = { message: 'Reading the flows...', runs: [], documents: [] }

// What panel.json asks for, read once at session.start: the pane opens unless
// `"pane": false`, the band draws only with `"band": true`.
let config = { pane: true, band: false }

async function readConfig($) {
  try {
    const panel = JSON.parse(await $.fs.read($.plugin.root + '/panel.json'))
    return { pane: panel.pane !== false, band: panel.band === true }
  } catch {
    return { pane: true, band: false } // no panel.json: the panel as before
  }
}

// The glyph of a step in the band: done, running, pending, skipped; `?` and `✗` as `flow
// status` shows them. Steps the coordinator holds or this session did not see are left out.
const BAND_SIGN = { done: '✓', running: '■', pending: '□', skipped: '⤼', unsure: '?', unexpected: '✗' }

// `deliver #menu  ✓ design  ✓ build  ■ guard  □ validate  ⤼ test`, in at most `columns`.
// Too long: the steps before the first one not done are folded into one `…`, then the
// end is cut with `…`. Claude Code wraps a longer line (and draws its `[-]` after the
// first row), so the band keeps itself narrower than `columns`.
// shortcut: the width is counted in UTF-16 units, not terminal cells, so a wide or
// ambiguous-width glyph (CJK in a step name, some emoji) can make a line wrap; improve
// it with a cell-width table when a flow names steps outside the Latin alphabet.
function bandText(run, columns) {
  const head = run.flow + (run.lot ? ' #' + run.lot : '')
  const steps = run.steps.filter(s => BAND_SIGN[s.mark]).map(s => BAND_SIGN[s.mark] + ' ' + s.step)
  const joined = parts => parts.join('  ')
  const room = Math.max(columns - 4, 10) // the `[-]` mark and a margin
  let first = 0
  const firstOpen = run.steps.filter(s => BAND_SIGN[s.mark]).findIndex(s => s.mark !== 'done')
  const open = firstOpen < 0 ? steps.length : firstOpen
  let text = joined([head, ...steps])
  while (text.length > room && first < open) {
    first += 1
    text = joined([head, '…', ...steps.slice(first)])
  }
  return text.length > room ? text.slice(0, room - 1) + '…' : text
}

// The .md files under `dir` (relative to the project root) with their mtime.
async function walk($, root, dir, depth) {
  let entries = []
  try {
    entries = await $.fs.list(root + '/' + dir)
  } catch {
    return [] // a folder that does not exist (yet) lists nothing
  }
  const found = []
  for (const entry of entries) {
    const path = dir + '/' + entry.name
    if (entry.kind === 'file' && entry.name.endsWith('.md')) {
      found.push({ path, mtimeMs: entry.mtimeMs })
    } else if (entry.kind === 'dir' && depth < MAX_DEPTH) {
      found.push(...(await walk($, root, path, depth + 1)))
    }
  }
  return found
}

async function documents($, root) {
  let folders = ['docs']
  try {
    const panel = JSON.parse(await $.fs.read($.plugin.root + '/panel.json'))
    if (Array.isArray(panel.documents)) folders = panel.documents
  } catch {
    // no panel.json: the default folder
  }
  const since = (await $.session.usage()).startedAt
  const all = []
  for (const folder of folders) all.push(...(await walk($, root, folder, 0)))
  return all
    .filter(doc => doc.mtimeMs >= since)
    .sort((a, b) => b.mtimeMs - a.mtimeMs)
    .slice(0, MAX_DOCUMENTS)
    .map(doc => doc.path)
}

async function status($, root) {
  // from the project root, wherever Claude Code was started in the project
  const { exitCode, stdout, stderr } = await $.process.run(['armadai', 'flow', 'status', '--json'], {
    cwd: root,
  })
  if (exitCode !== 0) throw new Error(stderr.trim() || 'exit code ' + exitCode)
  const json = JSON.parse(stdout)
  if (json.schema !== SCHEMA) {
    return {
      message:
        'armadai flow status --json gave schema ' +
        json.schema +
        '; this panel reads ' +
        SCHEMA +
        '. Run armadai link again.',
      runs: [],
    }
  }
  // right schema, wrong shape (no `runs`, a run without `steps`): read as nothing to draw
  const runs = (Array.isArray(json.runs) ? json.runs : [])
    .filter(run => run && typeof run === 'object')
    .map(run => ({ ...run, steps: Array.isArray(run.steps) ? run.steps : [] }))
  return { message: null, runs }
}

// True while a refresh runs, so a slow `armadai` is not launched again every 2 seconds.
let isRefreshing = false

async function refresh($) {
  if (isRefreshing) return
  isRefreshing = true
  try {
    const root = await $.session.root()
    let next
    try {
      next = await status($, root)
    } catch (error) {
      next = { message: 'armadai flow status --json failed: ' + error.message, runs: [] }
    }
    try {
      next.documents = config.pane ? await documents($, root) : []
    } catch {
      next.documents = []
    }
    view = next
    $.ui.invalidate('ui.render')
  } finally {
    isRefreshing = false
  }
}

export function register(on) {
  on('session.start', async ($, e, next) => {
    config = await readConfig($)
    if (config.pane) void $.ui.open({ id: PANE, title: 'ArmadAI' })
    void refresh($)
    $.clock.every(EVERY_MS, () => refresh($))
    if (config.pane) {
      try {
        await $.command.register({
          name: 'armadai-panel',
          description: 'Open the ArmadAI panel: flows and recent documents',
        })
      } catch {
        // a taken name throws, and a hook that throws is skipped: the panel works without it
      }
    }
    return next(e)
  })

  on('command.run', { command: 'armadai-panel' }, async $ => {
    await $.ui.open({ id: PANE, title: 'ArmadAI' })
    return { text: 'ArmadAI panel opened.' }
  })

  // the band: the open run (the latest one in progress), or nothing of ours
  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    const run = config.band ? view.runs.filter(r => r.status === 'in-progress').pop() : undefined
    if (!run) return next(e)
    const { Box, Text } = $.ui.resolve(e)
    const rest = await next(e) // what the mods after this one draw stays
    const line = h(Text, null, bandText(run, e.props.bodyColumns))
    return h(Box, { flexDirection: 'column' }, ...(rest ? [line, rest] : [line]))
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text } = $.ui.resolve(e)
    const rows = []
    if (view.message) rows.push(h(Text, { color: 'red' }, view.message))
    for (const run of view.runs) {
      rows.push(h(Text, { bold: true }, run.flow + ' (' + run.status + ')'))
      for (const step of run.steps) {
        rows.push(h(Text, null, step.sign + ' ' + step.step + (step.state ? ' (' + step.state + ')' : '')))
      }
    }
    if (!view.message && view.runs.length === 0) {
      rows.push(h(Text, { dimColor: true }, 'No run of a declared flow in this session yet.'))
    }
    rows.push(h(Text, { bold: true }, 'Documents'))
    if (view.documents.length === 0) rows.push(h(Text, { dimColor: true }, 'None changed in this session.'))
    for (const path of view.documents) rows.push(h(Text, null, path))
    return h(Box, { flexDirection: 'column' }, ...rows)
  })
}
