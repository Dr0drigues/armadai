# Extending ZSH_ENV

## Adding a New Function File

1. Create `~/.zsh_env/functions/my_feature.zsh`
2. It will be automatically sourced by `functions.zsh` at shell startup
3. Use the UI library for output (see `ui-system.md`)

```zsh
# ~/.zsh_env/functions/my_feature.zsh

my-command() {
    _ui_header "My Feature"
    # ...
    _ui_section "Status" "All good ${_ui_green}${_ui_check}${_ui_nc}"
}
```

### Lazy-loaded functions

For heavy files (>200 lines), add to the lazy loading system in `functions.zsh`:

```zsh
# In functions.zsh:
_ZSH_ENV_LAZY_FILES=(ai_context.zsh ai_tokens.zsh my_heavy.zsh)

# Create stubs for public functions:
for _fn in my_heavy_cmd1 my_heavy_cmd2; do
    eval "${_fn}() { _zsh_env_lazy_load my_heavy.zsh ${_fn} \"\$@\"; }"
done
```

## Adding Aliases

- **Global aliases**: Add to `aliases.zsh`
- **Personal aliases**: Add to `aliases.local.zsh` (gitignored)
- Always check tool existence: `if command -v tool &> /dev/null; then`

```zsh
if command -v my-tool &> /dev/null; then
    alias mt='my-tool'
    alias mtl='my-tool list'
fi
```

## Adding Completions

### For zsh-env commands

Add completion functions to `functions/zsh_env_completions.zsh`:

```zsh
_my_command() {
    _arguments \
        '1:subcommand:(list add remove)' \
        '*:args:_files'
}
compdef _my_command my-command
```

### For external tools

Use the custom completions registry in `completions.zsh`:

```zsh
_ZSH_ENV_CUSTOM_COMPLETIONS=(
    "armadai:armadai completion zsh"
    "my-tool:my-tool completions zsh"
)
```

Or use the CLI: `zsh-env-completion-add my-tool "my-tool completions zsh"`

**Important**: The tool must be in PATH when `rc.zsh` loads. If the tool is installed in a non-standard path (e.g., `~/.local/bin`), ensure that path is added to `$PATH` **before** `source "$ZSH_ENV_DIR/rc.zsh"` in `.zshrc`.

## Adding a Module

1. Add a flag in `config.zsh`:
   ```zsh
   ZSH_ENV_MODULE_MY_FEATURE=true
   ```

2. Set a default in `rc.zsh`:
   ```zsh
   ZSH_ENV_MODULE_MY_FEATURE=${ZSH_ENV_MODULE_MY_FEATURE:-false}
   ```

3. Guard module code with the flag:
   ```zsh
   if [[ "$ZSH_ENV_MODULE_MY_FEATURE" = "true" ]]; then
       # module-specific code
   fi
   ```

4. Add to `zsh-env-doctor` for diagnostics.

## Adding Starship Themes

1. Create `~/.zsh_env/themes/my-theme.toml`
2. First line should be: `# Starship Theme: My Theme Description`
3. Apply with: `zsh-env-theme my-theme`

## Adding Ghostty Themes

1. Create `~/.zsh_env/ghostty/themes/my-theme`
2. First line should be: `# Ghostty Theme: My Theme Description`
3. Apply with: `zsh-env-ghostty my-theme`
4. Deploy with: `zsh-env-ghostty sync`

## Adding Hooks for External Tools

Add tool initialization to `hooks.zsh`:

```zsh
# === MY TOOL ===
if command -v my-tool &> /dev/null; then
    eval "$(my-tool init zsh)"
fi
```

## Adding Plugins

In `config.zsh`:

```zsh
ZSH_ENV_PLUGINS_ORG=zsh-users
ZSH_ENV_PLUGINS=(
    zsh-syntax-highlighting
    zsh-autosuggestions
    other-org/other-plugin    # explicit org
)
```

## Conventions

- All function/alias files check tool existence with `command -v`
- Secrets go in `~/.secrets` or `~/.gitlab_secrets` (gitignored)
- User-local customizations go in `*.local.zsh` files (gitignored)
- Reload config with `ss` alias
- Use `zsh-env-doctor` to verify installation health
- Function names: `snake_case` for utilities, `kebab-case` for user commands
- Private/internal functions: prefix with `_` (e.g., `_ui_header`, `_zsh_env_lazy_load`)
