// The ArmadAI panel mod, written as is by `armadai link` for `ui.panel: true`.
// Do not edit it: change project.yaml and run `armadai link` again.
// It runs `armadai flow status --json` every 2 seconds and lists the Markdown files
// of the folders in panel.json (`ui.documents`) changed since the session began.
const PANE = 'armadai-flows'
const SCHEMA = 'armadai.flow-status/1'
const EVERY_MS = 2000
const MAX_DOCUMENTS = 20
const MAX_DEPTH = 4

// What the pane draws: set by `refresh`, read by `ui.render`.
let view = { message: 'Reading the flows...', runs: [], documents: [] }

// The .md files under `dir` (relative to the project) with their mtime.
async function walk($, dir, depth) {
  let entries = []
  try {
    entries = await $.fs.list(dir)
  } catch {
    return [] // a folder that does not exist (yet) lists nothing
  }
  const found = []
  for (const entry of entries) {
    const path = dir + '/' + entry.name
    if (entry.kind === 'file' && entry.name.endsWith('.md')) {
      found.push({ path, mtimeMs: entry.mtimeMs })
    } else if (entry.kind === 'dir' && depth < MAX_DEPTH) {
      found.push(...(await walk($, path, depth + 1)))
    }
  }
  return found
}

async function documents($) {
  let folders = ['docs']
  try {
    const panel = JSON.parse(await $.fs.read($.plugin.root + '/panel.json'))
    if (Array.isArray(panel.documents)) folders = panel.documents
  } catch {
    // no panel.json: the default folder
  }
  const since = (await $.session.usage()).startedAt
  const all = []
  for (const folder of folders) all.push(...(await walk($, folder, 0)))
  return all
    .filter(doc => doc.mtimeMs >= since)
    .sort((a, b) => b.mtimeMs - a.mtimeMs)
    .slice(0, MAX_DOCUMENTS)
    .map(doc => doc.path)
}

async function status($) {
  const { stdout } = await $.process.run(['armadai', 'flow', 'status', '--json'])
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
  return { message: null, runs: json.runs }
}

async function refresh($) {
  let next
  try {
    next = await status($)
  } catch (error) {
    next = { message: 'armadai flow status --json failed: ' + error.message, runs: [] }
  }
  try {
    next.documents = await documents($)
  } catch {
    next.documents = []
  }
  view = next
  $.ui.invalidate('ui.render')
}

export function register(on) {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'armadai-panel',
      description: 'Open the ArmadAI panel: flows and recent documents',
    })
    void $.ui.open({ id: PANE, title: 'ArmadAI' })
    void refresh($)
    $.clock.every(EVERY_MS, () => refresh($))
    return next(e)
  })

  on('command.run', { command: 'armadai-panel' }, async $ => {
    await $.ui.open({ id: PANE, title: 'ArmadAI' })
    return { text: 'ArmadAI panel opened.' }
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
