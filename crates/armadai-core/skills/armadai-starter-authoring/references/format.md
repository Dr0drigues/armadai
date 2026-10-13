# Starter pack format

## Folder

```
my-pack/
  pack.yaml               # required
  project.yaml            # optional: targets, coordinator, policy
  agents/<id>.md
  skills/<id>/SKILL.md    # and the rest of each skill folder
  flows/<id>.yaml
  automations/<id>.yaml
```

## pack.yaml

```yaml
format: 1
name: my-pack
description: "What the team does, in one sentence"
```

- `format: 1` makes the pack native. Without `format`, the pack is the legacy format (an
  `agents:` list, `prompts/`), still converted by `migrate` at install, whatever its origin.
  Another value is refused.
- `name` and `description` are listed by `armadai init` and the registries.

## What `armadai init --pack` does

1. Refuses a folder that already holds `.armadai/project.yaml` (without `--force`).
2. Refuses a pack holding any symbolic link.
3. Validates the pack in a scratch project; any error stops the install, nothing written.
4. Copies, by a white list: `project.yaml`, `agents/*.md`, `skills/**`, `flows/*.yaml`,
   `automations/*.yaml`. Anything else (`pack.yaml`, a README, `.armadai-version`) stays behind.
5. Keeps an existing file (one warning each); an existing `project.yaml` wins over the pack's
   (its targets, coordinator and policy are then not applied: said so).
6. Writes `project.yaml` (`schema_version: 1`, `targets: [claude]`) when neither has one.
7. Lists every automation of the pack: it becomes a hook the target runs.
8. Undoes everything if the result has an error the project did not have.

## Where packs are found

`armadai init --pack <name>` looks in: the packs built into the binary, `.armadai/starters/`,
`~/.config/armadai/starters/`, `ARMADAI_STARTERS_DIRS`, `starters_dirs` of the user config, then
the synced remote registries. `armadai init --pack <path>` takes a folder directly.

## Checking a pack

```bash
armadai validate path/to/my-pack          # validates the tree as a project
armadai init --pack path/to/my-pack       # in a scratch folder
armadai link
```
