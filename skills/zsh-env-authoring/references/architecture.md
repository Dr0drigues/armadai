# ZSH_ENV Architecture

## Directory Layout

```
~/.zsh_env/
├── rc.zsh              # Entry point (sourced by .zshrc)
├── config.zsh          # User configuration (modules, plugins, update settings)
├── variables.zsh       # Environment variables ($WORK_DIR, $SCRIPTS_DIR, history)
├── completions.zsh     # Custom completions registry (_ZSH_ENV_CUSTOM_COMPLETIONS)
├── aliases.zsh         # Global aliases (ls, git, system utils)
├── aliases.local.zsh   # User-local aliases (gitignored)
├── functions.zsh       # Dynamic loader for functions/ directory
├── hooks.zsh           # External tool hooks (starship, mise, zoxide, direnv, fzf)
├── plugins.zsh         # Lightweight plugin manager (git-based)
├── functions/          # Modular function files
│   ├── ui.zsh          # UI library (loaded first) — colors, formatters, symbols
│   ├── zsh_env_commands.zsh   # zsh-env-* commands (list, doctor, status, help, etc.)
│   ├── zsh_env_completions.zsh # Completion functions for zsh-env commands
│   ├── ai_context.zsh  # AI context generation (lazy-loaded)
│   ├── ai_tokens.zsh   # AI token estimation (lazy-loaded)
│   ├── docker_utils.zsh
│   ├── gitlab_logic.zsh
│   ├── kube_config.zsh
│   ├── security_audit.zsh
│   ├── ssh_manager.zsh
│   ├── project_switcher.zsh
│   └── ...
├── scripts/            # Standalone shell scripts
├── themes/             # Starship theme .toml files
├── ghostty/            # Ghostty terminal config and themes
├── plugins/            # Auto-managed plugin repos (git clones)
├── boulanger/          # Enterprise-specific module (Boulanger context)
└── install.sh          # Cross-platform bootstrapper
```

## Loading Order

Defined in `rc.zsh`, the loading order is strict:

1. **Configuration** (`config.zsh`) — Module flags, plugin list, update settings
2. **Secrets** (`~/.secrets`) — API tokens, credentials (not in repo)
3. **Variables** (`variables.zsh`) — PATH additions, history config, SSL certs
4. **Completions** — `compinit` + zstyle + custom completions from `completions.zsh`
5. **Functions** (`functions.zsh`) — Loads `ui.zsh` first, then all other `functions/*.zsh`
6. **Aliases** (`aliases.zsh` + `aliases.local.zsh`)
7. **Plugins** (`plugins.zsh`) — Git-cloned plugins from `ZSH_ENV_PLUGINS` array
8. **Hooks** (`hooks.zsh`) — External tool init (starship, mise, zoxide, direnv, fzf)

### Important: PATH and completions

Custom completions in `completions.zsh` use `command -v` to check if a tool exists before loading its completion. Any PATH additions for tools that need completions **must** happen before `rc.zsh` is sourced (i.e., in `.zshrc` before the `source` line, or in `variables.zsh`).

## Module System

Modules are toggled via boolean flags in `config.zsh`:

```zsh
ZSH_ENV_MODULE_GITLAB=true    # GitLab functions and aliases
ZSH_ENV_MODULE_DOCKER=true    # Docker utilities
ZSH_ENV_MODULE_MISE=true      # mise version manager hooks
ZSH_ENV_MODULE_NUSHELL=true   # Nushell integration
ZSH_ENV_MODULE_KUBE=true      # Kubernetes tools
```

Module-dependent code checks these flags: `[[ "$ZSH_ENV_MODULE_DOCKER" = "true" ]]`.

## Plugin Manager

Lightweight git-based plugin manager in `plugins.zsh`:

```zsh
ZSH_ENV_PLUGINS_ORG=zsh-users   # Default GitHub org
ZSH_ENV_PLUGINS=(
    zsh-syntax-highlighting      # → zsh-users/zsh-syntax-highlighting
    zsh-autosuggestions          # → zsh-users/zsh-autosuggestions
    Aloxaf/fzf-tab               # Explicit org
)
```

Plugins are cloned to `~/.zsh_env/plugins/` and sourced automatically.

## Custom Completions Registry

`completions.zsh` defines an array of `"name:command"` entries:

```zsh
_ZSH_ENV_CUSTOM_COMPLETIONS=(
    "armadai:armadai completion zsh"
    "bun:bun completions"
)
```

At startup, `rc.zsh` iterates the array, checks `command -v "$name"`, and `eval`s the command output. Manage with `zsh-env-completion-add` and `zsh-env-completion-remove`.

## Lazy Loading

Heavy function files are loaded on first call via stubs in `functions.zsh`:

```zsh
_ZSH_ENV_LAZY_FILES=(ai_context.zsh ai_tokens.zsh)

_zsh_env_lazy_load() {
    local file="$1"; shift; local func="$1"
    source "$ZSH_ENV_DIR/functions/$file"
    "$func" "$@"
}

# Creates stub: ai_context_detect() calls _zsh_env_lazy_load on first invocation
for _fn in ai_context_detect ai_context_init ...; do
    eval "${_fn}() { _zsh_env_lazy_load ai_context.zsh ${_fn} \"\$@\"; }"
done
```

## Auto-Update

Controlled by `config.zsh`:

```zsh
ZSH_ENV_AUTO_UPDATE=true
ZSH_ENV_UPDATE_FREQUENCY=7    # days
ZSH_ENV_UPDATE_MODE="prompt"  # "prompt" or "auto"
```

## Key Commands

| Command | Description |
|---------|-------------|
| `ss` | Reload shell config (`source ~/.zshrc`) |
| `zsh-env-doctor` | Full diagnostic of installation |
| `zsh-env-status` | Quick status overview |
| `zsh-env-list` | List installed tools with versions |
| `zsh-env-completions` | Load all completions interactively |
| `zsh-env-completion-add` | Register a new completion |
| `zsh-env-completion-remove` | Remove a completion |
| `zsh-env-theme [name]` | List/apply Starship themes |
| `zsh-env-ghostty [name\|sync]` | List/apply/sync Ghostty themes |
| `zsh-env-ssl-setup` | Configure enterprise SSL certs |
| `zsh-env-update` | Update zsh_env from git |
| `zsh-env-help` | Show help |
