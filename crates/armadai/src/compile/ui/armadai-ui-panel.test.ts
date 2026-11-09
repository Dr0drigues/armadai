// Not shipped: run with `claude plugin test` from a folder `armadai link` wrote for
// `ui.panel: true` (copy this file next to hooks/). CI has no Claude Code, so the
// validator runs it at the real trial.
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
) {
  const clock = mock.clock(on)
  on('command.register', () => (isCommandTaken ? { deny: 'taken' } : { value: undefined }))
  // the project root is not the folder Claude Code was started in
  on('session.root', () => ({ value: '/p' }))
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('session.start', (_: any, e: any) => ({ cwd: e.cwd }))
  on('process.run', async (_: any, e: any) => {
    expect(e.argv).toEqual(['armadai', 'flow', 'status', '--json'])
    expect(e.init?.cwd).toBe('/p')
    if (run) return await run(clock)
    return { value: { exitCode: 0, stdout: STATUS(schema), stderr: '' } }
  })
  on('fs.read', (_: any, e: any) => {
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
