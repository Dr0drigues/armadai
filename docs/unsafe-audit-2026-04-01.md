# Unsafe Code Audit - April 1, 2026

## Summary

Comprehensive audit of all `unsafe` blocks in the ArmadAI codebase. All unsafe code has been documented with `// SAFETY:` comments explaining the rationale and constraints.

## Findings

### Total Unsafe Blocks: 15

All located in test code, none in production code.

### Breakdown by File

#### `/Users/bl209054/work/misc/armadai/src/core/config.rs` (8 blocks)

**Lines 427-439** - `test_config_dir_respects_env`
- **Usage**: `std::env::set_var()` and `std::env::remove_var()`
- **Reason**: Testing config directory resolution with environment variables
- **Safety**: Requires `--test-threads=1` to avoid data races with `test_user_dirs`
- **Status**: ✅ Documented, justified

**Lines 474-501** - `test_env_overrides`
- **Usage**: `std::env::set_var()` for ARMADAI_PROVIDER, ARMADAI_MODEL, ARMADAI_TEMPERATURE
- **Reason**: Testing environment variable overrides for user config
- **Safety**: Safe for single-threaded test runs
- **Status**: ✅ Documented, justified

**Lines 507-523** - `test_user_dirs`
- **Usage**: `std::env::set_var()` and `std::env::remove_var()`
- **Reason**: Testing user directory resolution
- **Safety**: Requires `--test-threads=1` to avoid data races with `test_config_dir_respects_env`
- **Status**: ✅ Documented, justified

#### `/Users/bl209054/work/misc/armadai/src/core/skill.rs` (2 blocks)

**Lines 299-341** - `test_install_embedded_skills`
- **Usage**: `std::env::set_var()` and `std::env::remove_var()` for ARMADAI_CONFIG_DIR
- **Reason**: Testing embedded skills installation with custom config directory
- **Safety**: Requires `--test-threads=1` to avoid data races with tests in core::config and core::starter
- **Status**: ✅ Documented, justified

#### `/Users/bl209054/work/misc/armadai/src/core/starter.rs` (5 blocks)

**Lines 406-491** - `test_install_pack`
- **Usage**: `std::env::set_var()` and `std::env::remove_var()` for ARMADAI_CONFIG_DIR
- **Reason**: Testing starter pack installation with custom config directory
- **Safety**: Requires `--test-threads=1` to avoid data races with tests in core::config and core::skill
- **Status**: ✅ Documented, justified

**Lines 558-585** - `test_env_var_starters_dirs`
- **Usage**: `std::env::set_var()` and `std::env::remove_var()` for ARMADAI_STARTERS_DIRS
- **Reason**: Testing custom starters directories from environment variable
- **Safety**: Only test touching ARMADAI_STARTERS_DIRS, still requires `--test-threads=1` for safety
- **Status**: ✅ Documented, justified

## Rationale

All `unsafe` blocks use `std::env::set_var()` and `std::env::remove_var()`, which are marked `unsafe` in Rust edition 2024 because:

1. **Data Race Risk**: Environment variables are global mutable state. In multi-threaded programs, concurrent access can cause undefined behavior.
2. **Thread Safety**: Cargo runs tests in parallel by default, which can cause races when multiple tests modify the same environment variables.

## Mitigation

### Current Approach

- All unsafe blocks are properly documented with `// SAFETY:` comments
- Comments explain the data race risk and mitigation strategy
- Tests restore original environment state after execution

### Recommended Test Execution

For tests that modify environment variables, use sequential execution:

```bash
cargo test --no-default-features --features tui,providers-api -- --test-threads=1
```

### Alternative Solutions Considered

1. **Use `#[serial]` attribute**: Would require adding `serial_test` crate dependency
2. **Mock environment**: Would require refactoring to inject environment access
3. **Accept flakiness**: Not acceptable for reliable CI

### Decision

Keep current approach (documented unsafe + sequential test execution) because:
- Simple and straightforward
- No additional dependencies
- Tests are isolated and don't affect production code
- CI can enforce `--test-threads=1` for environment-dependent tests

## Production Code

**Result**: ✅ **ZERO** unsafe blocks in production code

All unsafe code is confined to test modules, which significantly reduces risk.

## Verification

```bash
# Clippy passes with all warnings as errors
cargo clippy --no-default-features --features tui,providers-api -- -D warnings

# All tests pass in sequential mode
cargo test --no-default-features --features tui,providers-api -- --test-threads=1
```

## Recommendations

1. ✅ **Document all unsafe blocks** - COMPLETED
2. ⚠️ **CI enforcement**: Consider adding `--test-threads=1` to CI for environment tests
3. 💡 **Future**: Consider refactoring to use test fixtures that don't require env var mutation
4. 💡 **Future**: Add `serial_test` crate if env tests grow significantly

## Files Modified

- `/Users/bl209054/work/misc/armadai/src/core/config.rs` - Added SAFETY comments to 8 unsafe blocks
- `/Users/bl209054/work/misc/armadai/src/core/skill.rs` - Added SAFETY comments to 2 unsafe blocks
- `/Users/bl209054/work/misc/armadai/src/core/starter.rs` - Added SAFETY comments to 5 unsafe blocks

## Audit Performed By

Core Specialist (ArmadAI Dev Team)  
Date: April 1, 2026  
Rust Edition: 2024
