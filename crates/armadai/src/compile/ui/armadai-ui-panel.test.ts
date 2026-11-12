// Not shipped: run with `claude plugin test` from a folder `armadai link` wrote for
// `ui.panel: true` or `ui.flow_band: true` (copy this file next to hooks/). CI has no
// Claude Code, so the validator runs it at the real trial.
import { expect, mock, test } from 'claude-code/testing'

const PANE = {
  plugin: 'armadai-ui',
  component: 'Pane',
  requestId: 'armadai-flows',
  surface: 'terminal',
  props: {
    title: 'ArmadAI',
    isFocused: false,
    bodyColumns: 60,
    placement: 'dock',
    scroll: { offset: 0, bodyRows: 20 },
    view: {},
  },
} as const

const BAND = (columns: number) =>
  ({
    plugin: 'armadai-ui',
    component: 'AbovePrompt',
    requestId: 'band',
    surface: 'terminal',
    viewport: { columns, rows: 30 },
    props: {
      hasSurvey: false,
      isWorking: false,
      maxRows: 17,
      bodyColumns: columns,
      scroll: { offset: 0, bodyRows: 17 },
      view: {},
    },
  }) as const

const MARK_SIGN: any = { done: '✓', running: '●', pending: '○', skipped: '⤼', coordinator: '~' }
// a run in progress; `steps` are [name, mark]
const BAND_STATUS = (steps: [string, string][], status = 'in-progress', lot: string | null = 'menu') => async () => ({
  value: {
    exitCode: 0,
    stderr: '',
    stdout: JSON.stringify({
      schema: 'armadai.flow-status/1',
      session: { id: 's1', started_at: null, last_event_at: null },
      runs: [
        {
          flow: 'deliver',
          lot,
          status,
          steps: steps.map(([step, mark]) => ({ step, mark, sign: MARK_SIGN[mark], state: null, note: null })),
          loops: [],
          transitions: [],
        },
      ],
    }),
  },
})
const FIVE: [string, string][] = [
  ['frame', 'coordinator'],
  ['design', 'done'],
  ['build', 'done'],
  ['guard', 'running'],
  ['validate', 'pending'],
  ['test', 'skipped'],
]

const STATUS = (schema: string) =>
  JSON.stringify({
    schema,
    session: { id: 's1', started_at: null, last_event_at: null },
    runs: [
      {
        flow: 'deliver',
        status: 'in-progress',
        steps: [
          { step: 'design', mark: 'done', sign: '✓', state: 'done', note: null },
          { step: 'guard', mark: 'running', sign: '●', state: null, note: null },
        ],
        loops: [],
        transitions: [],
      },
    ],
  })

const SINCE = 1_000_000
const file = (name: string, mtimeMs: number) => ({ name, kind: 'file', size: 1, mtimeMs, isLink: false })

async function start(
  $: any,
  on: any,
  schema: string,
  documents: string[] | null,
  run: any = null,
  isCommandTaken = false,
  panelJson: any = undefined,
  opened: string[] = [],
) {
  const clock = mock.clock(on)
  on('command.register', () => (isCommandTaken ? { deny: 'taken' } : { value: undefined }))
  // the project root is not the folder Claude Code was started in
  on('session.root', () => ({ value: '/p' }))
  on('ui.open', (_: any, e: any) => {
    opened.push(e.id)
    return { value: { isPlaced: true } }
  })
  // what Claude Code draws in the band when the mod passes with next(e)
  on('ui.render', () => ({ type: 'Text', props: {}, children: ['drawn by Claude Code'] }))
  on('session.start', (_: any, e: any) => ({ cwd: e.cwd }))
  on('process.run', async (_: any, e: any) => {
    expect(e.argv).toEqual(['armadai', 'flow', 'status', '--json'])
    expect(e.init?.cwd).toBe('/p')
    if (run) return await run(clock)
    return { value: { exitCode: 0, stdout: STATUS(schema), stderr: '' } }
  })
  on('fs.read', (_: any, e: any) => {
    if (panelJson !== undefined) return { value: JSON.stringify(panelJson) }
    if (documents === null) return { deny: 'no panel.json' }
    return { value: JSON.stringify({ documents }) }
  })
  on('fs.list', (_: any, e: any) => {
    expect(e.path.startsWith('/p/')).toBe(true)
    return {
    value:
      e.path.endsWith('/notes')
        ? [file('new.md', SINCE + 5), file('old.md', SINCE - 5), file('b.txt', SINCE + 9)]
        : e.path.endsWith('/docs')
          ? [file('d.md', SINCE + 1)]
          : [],
    }
  })
  on('session.usage', () => ({ value: { startedAt: SINCE, context: {}, rateLimits: [], cost: {} } }))
  await $.session.start({ surface: 'terminal', isInteractive: true, cwd: '/p' })
  await clock.advance(10)
  return await $.ui.mount(PANE)
}

// the band alone (`ui.flow_band` without `ui.panel`), drawn at `columns`
async function band($: any, on: any, steps: [string, string][], columns: number, panelJson: any, status = 'in-progress') {
  const opened: string[] = []
  const clock = mock.clock(on)
  on('command.register', () => ({ value: undefined }))
  on('session.root', () => ({ value: '/p' }))
  on('ui.open', (_: any, e: any) => {
    opened.push(e.id)
    return { value: { isPlaced: true } }
  })
  on('ui.render', () => ({ type: 'Text', props: {}, children: ['drawn by Claude Code'] }))
  on('session.start', (_: any, e: any) => ({ cwd: e.cwd }))
  on('process.run', BAND_STATUS(steps, status))
  on('fs.read', () => ({ value: JSON.stringify(panelJson) }))
  on('fs.list', () => ({ value: [] }))
  on('session.usage', () => ({ value: { startedAt: SINCE, context: {}, rateLimits: [], cost: {} } }))
  await $.session.start({ surface: 'terminal', isInteractive: true, cwd: '/p' })
  await clock.advance(10)
  return { ui: await $.ui.mount(BAND(columns)), opened }
}

test('a run is listed with its steps and the documents of ui.documents, newest first', async ($, on) => {
  const ui = await start($, on, 'armadai.flow-status/1', ['docs', 'notes'])
  expect(await ui.find({ type: 'Text', text: 'deliver (in-progress)' })).toBeDefined()
  expect(await ui.find({ type: 'Text', text: '✓ design (done)' })).toBeDefined()
  expect(await ui.find({ type: 'Text', text: '● guard' })).toBeDefined()
  expect(await ui.find({ type: 'Text', text: 'notes/new.md' })).toBeDefined()
  expect(await ui.find({ type: 'Text', text: 'docs/d.md' })).toBeDefined()
  expect((await ui.find({ type: 'Box' }))?.text).toContain('notes/new.mddocs/d.md')
  // older than the session, and not Markdown: not listed
  expect(await ui.find({ type: 'Text', text: 'notes/old.md' })).toBeUndefined()
  expect(await ui.find({ type: 'Text', text: 'notes/b.txt' })).toBeUndefined()
  await ui.unmount()
})

test('without panel.json the default folder docs is read', async ($, on) => {
  const ui = await start($, on, 'armadai.flow-status/1', null)
  expect(await ui.find({ type: 'Text', text: 'docs/d.md' })).toBeDefined()
  expect(await ui.find({ type: 'Text', text: 'notes/new.md' })).toBeUndefined()
  await ui.unmount()
})

test('another schema is refused with the message, and no run is drawn', async ($, on) => {
  const ui = await start($, on, 'armadai.flow-status/2', ['docs'])
  expect(
    await ui.find({
      type: 'Text',
      text: 'armadai flow status --json gave schema armadai.flow-status/2; this panel reads armadai.flow-status/1. Run armadai link again.',
    }),
  ).toBeDefined()
  expect(await ui.find({ type: 'Text', text: 'deliver (in-progress)' })).toBeUndefined()
  await ui.unmount()
})

test('a failing armadai shows its stderr, not the JSON error', async ($, on) => {
  const ui = await start($, on, 'armadai.flow-status/1', ['docs'], async () => ({
    value: { exitCode: 2, stdout: '', stderr: 'error: unrecognized subcommand\n' },
  }))
  expect(
    await ui.find({
      type: 'Text',
      text: 'armadai flow status --json failed: error: unrecognized subcommand',
    }),
  ).toBeDefined()
  await ui.unmount()
})

test('a slow armadai is not launched again while it runs', async ($, on) => {
  let calls = 0
  let time: any
  await start($, on, 'armadai.flow-status/1', ['docs'], async (clock: any) => {
    time = clock
    calls += 1
    await clock.sleep(5000)
    return { value: { exitCode: 0, stdout: STATUS('armadai.flow-status/1'), stderr: '' } }
  })
  // two more periods of 2 s pass while the first call still runs
  await time.advance(4000)
  expect(calls).toBe(1)
})

test('a taken command name does not stop the panel or the timer', async ($, on) => {
  let calls = 0
  let time: any
  const ui = await start(
    $,
    on,
    'armadai.flow-status/1',
    ['docs'],
    async (clock: any) => {
      time = clock
      calls += 1
      return { value: { exitCode: 0, stdout: STATUS('armadai.flow-status/1'), stderr: '' } }
    },
    true,
  )
  expect(await ui.find({ type: 'Text', text: 'deliver (in-progress)' })).toBeDefined()
  await time.advance(2000)
  expect(calls).toBeGreaterThan(1)
  await ui.unmount()
})

const LINE = 'deliver #menu  ✓ design  ✓ build  ■ guard  □ validate  ⤼ test'

test('the band alone draws the open run on one line and opens no pane', async ($, on) => {
  const { ui, opened } = await band($, on, FIVE, 100, { band: true, pane: false })
  expect(await ui.find({ type: 'Text', text: LINE })).toBeDefined()
  // Claude Code's own drawing for the band stays under ours
  expect(await ui.find({ type: 'Text', text: 'drawn by Claude Code' })).toBeDefined()
  expect(opened).toEqual([])
  await ui.unmount()
})

const LONG: [string, string][] = [
  ...['analysis', 'design', 'build', 'review', 'verify'].map((s): [string, string] => [s, 'done']),
  ['guard', 'running'],
  ['validate', 'pending'],
  ['test', 'skipped'],
]

for (const columns of [80, 50]) {
  test(`at ${columns} columns a band that does not fit folds the steps done and stays inside`, async ($, on) => {
    const { ui } = await band($, on, LONG, columns, { band: true, pane: false })
    const text = (await ui.find({ type: 'Text', text: /■ guard/ }))?.children[0] as string
    expect(text.length).toBeLessThanOrEqual(columns - 4)
    expect(text.startsWith('deliver #menu  …  ')).toBe(true)
    expect(text.endsWith('■ guard  □ validate  ⤼ test')).toBe(true)
    await ui.unmount()
  })
}

test('a band narrower than its open steps is cut with an ellipsis', async ($, on) => {
  const { ui } = await band($, on, FIVE, 20, { band: true, pane: false })
  expect((await ui.find({ type: 'Text', text: /^deliver/ }))?.children[0]).toBe('deliver #menu  …')
  await ui.unmount()
})

test('without a run in progress, or without band in panel.json, nothing of ours is drawn', async ($, on) => {
  const done = await band($, on, FIVE, 100, { band: true, pane: false }, 'finished')
  expect(await done.ui.find({ type: 'Text', text: /deliver/ })).toBeUndefined()
  expect(await done.ui.find({ type: 'Text', text: 'drawn by Claude Code' })).toBeDefined()
  await done.ui.unmount()
})

test('a panel without band in panel.json draws no band', async ($, on) => {
  const { ui, opened } = await band($, on, FIVE, 100, { documents: ['docs'] })
  expect(await ui.find({ type: 'Text', text: /deliver/ })).toBeUndefined()
  // the pane opens as before
  expect(opened).toEqual(['armadai-flows'])
  await ui.unmount()
})

test('the panel shows a skipped step with its glyph', async ($, on) => {
  const opened: string[] = []
  const ui = await start($, on, 'armadai.flow-status/1', ['docs'], BAND_STATUS(FIVE), false, undefined, opened)
  expect(await ui.find({ type: 'Text', text: '⤼ test' })).toBeDefined()
  await ui.unmount()
})
