# ZSH_ENV UI System

The UI library (`functions/ui.zsh`) provides consistent formatting for all `zsh-env-*` commands. **All new commands must use these functions** — never hardcode ANSI escape sequences.

## Version

```zsh
export ZSH_ENV_VERSION="v1.7.0"
```

## Color Variables

```zsh
$_ui_green    $_ui_red    $_ui_yellow    $_ui_blue
$_ui_cyan     $_ui_magenta  $_ui_white   $_ui_black
$_ui_bold     $_ui_dim    $_ui_italic    $_ui_underline
$_ui_nc       # Reset / No Color
```

Legacy aliases (for backward compatibility):
```zsh
$_zsh_cmd_green  $_zsh_cmd_red  $_zsh_cmd_yellow
$_zsh_cmd_cyan   $_zsh_cmd_bold $_zsh_cmd_dim  $_zsh_cmd_nc
```

## Symbols

```zsh
$_ui_check    # ✓
$_ui_cross    # ✗
$_ui_circle   # ○
```

## Layout Functions

### `_ui_header "Title"`
Boxed header with version number. Use at the start of every `zsh-env-*` command.

```
╭─ Title ────────────── v1.7.0 ─╮
```

### `_ui_section "Label" content`
Section with a 14-character aligned label. Use for key-value display.

```zsh
_ui_section "Modules"  "GitLab ✓  Docker ✓  Mise ✓"
# Output:
#   Modules       GitLab ✓  Docker ✓  Mise ✓
```

### `_ui_separator [width]`
Horizontal line separator. Default width: 44.

### `_ui_summary $issues $warnings`
Final summary line with issue/warning counts.

## Inline Indicators

For compact status lines showing tool/feature availability:

```zsh
_ui_ok "docker" "24.0"     # docker ✓24.0
_ui_ok "docker"             # docker ✓
_ui_fail "docker" "missing" # docker ✗missing
_ui_fail "docker"           # docker ✗
_ui_warn "docker"           # docker ○
_ui_skip "docker"           # docker ○  (dim)
```

## Message Functions

Single-line status messages:

```zsh
_ui_msg_ok "Installation complete"    # ✓ Installation complete
_ui_msg_fail "File not found"         # ✗ File not found
_ui_tag_ok "SSL configured"           # [OK] SSL configured
```

## Utility Functions

```zsh
_ui_get_perms "/path/to/file"   # Cross-platform file permissions
_ui_truncate "long text" 20      # Truncate with "..."
```

## Example: Writing a New Command

```zsh
zsh-env-my-command() {
    _ui_header "My Command"

    local issues=0
    local warnings=0

    # Check something
    local status=""
    if command -v mytool &> /dev/null; then
        status+="mytool ${_ui_green}${_ui_check}${_ui_nc}  "
    else
        status+="mytool ${_ui_red}${_ui_cross}${_ui_nc}  "
        ((issues++))
    fi
    _ui_section "Tools" "$status"

    echo ""
    _ui_separator
    _ui_summary $issues $warnings
}
```

## Rules

1. **Always use `_ui_*` functions** for colors and formatting
2. **Never hardcode ANSI codes** (`\033[...`) in new files
3. Commands must start with `_ui_header "Title"`
4. Use `_ui_section` for aligned labels (14 chars)
5. End with `_ui_separator` + `_ui_summary` or a summary line
6. Keep output compact — group related info on one line
7. Use inline indicators (`_ui_ok`/`_ui_fail`) for tool checks
8. No excessive blank lines
