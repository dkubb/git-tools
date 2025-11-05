# Code Audit Report: git-tools

**Audit Date:** November 5, 2025
**Auditor:** Claude Code
**Codebase:** git-tools (collection of Git utility scripts)

---

## Executive Summary

This audit reviewed a collection of 12 Git utility scripts written in Rust using rust-script. The codebase provides various Git workflow automation tools. Overall, the code is well-structured and functional, but several issues were identified ranging from critical bugs to code quality improvements.

**Critical Issues:** 1
**High Priority:** 3
**Medium Priority:** 6
**Low Priority:** 4

---

## Critical Issues

### 1. Missing Dependency Declaration in git-prune-all
**File:** `git-prune-all:10`
**Severity:** CRITICAL
**Status:** Active Bug

The script imports `regex::Regex` on line 10, but the cargo dependencies block (lines 2-4) does not declare the `regex` crate:

```rust
//! ```cargo
//! [dependencies]
//! clap = { version = "4.5.48", features = ["derive"] }
//! ```

use regex::Regex;  // Line 10 - ERROR: regex crate not declared
```

However, upon further inspection, the code no longer uses the Regex import anywhere in the file. The import statement appears to be dead code.

**Impact:** The script will fail to compile when executed.

**Recommendation:** Remove the unused `use regex::Regex;` import statement from line 10.

---

## High Priority Issues

### 1. Deprecated git filter-branch Usage
**File:** `git-fix-branch:140-149`
**Severity:** HIGH
**Status:** Using Deprecated Tool

The script uses `git filter-branch` which Git itself warns is deprecated:

```rust
let status = Command::new("git")
    .args(&[
        "filter-branch",
        "--force",
        "--env-filter",
        env_filter,
        "--",
        &range,
    ])
```

**Impact:**
- Git displays deprecation warnings
- May be removed in future Git versions
- Performance is slower than modern alternatives
- Can corrupt repositories if interrupted

**Recommendation:** Migrate to `git filter-repo` or use `git rebase` with custom hooks. Example:
```bash
git rebase --exec 'git commit --amend --reset-author --no-edit' <parent>
```

### 2. Hardcoded Path Dependency
**File:** `git-reword-commit:165-167`
**Severity:** HIGH
**Status:** Fragile Implementation

The script assumes `git-new-from` is in the current working directory:

```rust
let git_new_from = std::env::current_dir()
    .expect("Failed to get current directory")
    .join("git-new-from");
```

**Impact:**
- Fails if executed from a different directory
- Not portable across different installation methods
- Breaks if tools are installed to PATH

**Recommendation:**
1. Use `which git-new-from` to find the tool in PATH
2. Or use the same directory as the current executable
3. Or make it a configuration option

Example fix:
```rust
let git_new_from = std::env::current_exe()
    .ok()
    .and_then(|p| p.parent().map(|d| d.join("git-new-from")))
    .or_else(|| which::which("git-new-from").ok())
    .expect("Failed to find git-new-from");
```

### 3. Excessive Use of expect() Leading to Panics
**Files:** Multiple files (21 instances found)
**Severity:** HIGH
**Status:** Poor Error Handling

Many scripts use `.expect()` for operations that could fail gracefully:

**Examples:**
- `git-prune-all:35` - Git rev-parse check
- `git-fix:770` - Temp file creation
- `git-reword-commit:166` - Current directory retrieval

**Impact:**
- Ungraceful failures with panic stack traces
- Poor user experience
- Cannot be caught by calling code

**Recommendation:** Replace `.expect()` with proper error handling using `Result` types or custom error messages. For example:

```rust
// Before
let status = Command::new("git")
    .args(&["rev-parse", "--git-dir"])
    .status()
    .expect("Failed to execute git rev-parse");

// After
let status = Command::new("git")
    .args(&["rev-parse", "--git-dir"])
    .status()
    .unwrap_or_else(|e| {
        eprintln!("✗ ERROR: Failed to check if directory is a git repository");
        eprintln!("DETAILS: {}", e);
        exit(EXIT_SOFTWARE);
    });
```

---

## Medium Priority Issues

### 1. Significant Code Duplication
**Files:** All script files
**Severity:** MEDIUM
**Status:** Maintainability Issue

The following functions are duplicated across 8+ files:
- `git_output()` - Duplicated 8 times
- `git_output_optional()` - Duplicated 7 times
- `resolve_default_branch()` - Duplicated 6 times
- `branch_exists()` - Duplicated 5 times
- `GitRef` struct and implementation - Duplicated 8 times

**Impact:**
- Bug fixes must be applied to multiple files
- Inconsistent implementations
- Larger codebase
- Higher maintenance burden

**Recommendation:**
1. Create a shared library crate for common functionality
2. Use a `common.rs` module with rust-script's `--extern` feature
3. Or accept the duplication as a tradeoff for single-file scripts

### 2. No Input Validation for File Paths in Some Commands
**Files:** Various
**Severity:** MEDIUM
**Status:** Potential Security Issue

While most scripts properly validate git references, some commands accept file paths without thorough validation:

**Example in git-new-from:94-95:**
```rust
#[arg(long, short = 'F')]
file: Option<String>,
```

The file path is passed directly to git commands without checking for:
- Path traversal attempts
- Existence verification
- Permission checks

**Impact:**
- Could read sensitive files if permissions allow
- Error messages might leak filesystem structure
- Confusing errors for users

**Recommendation:** Add path validation before use:
```rust
if let Some(ref path) = file {
    let p = Path::new(path);
    if !p.exists() {
        eprintln!("✗ ERROR: File not found: {}", path);
        exit(EXIT_DATAERR);
    }
    if !p.is_file() {
        eprintln!("✗ ERROR: Path is not a file: {}", path);
        exit(EXIT_DATAERR);
    }
}
```

### 3. Race Conditions in File Operations
**File:** `git-sync-mtime:141-144`
**Severity:** MEDIUM
**Status:** TOCTOU Vulnerability

Time-of-check to time-of-use (TOCTOU) race condition:

```rust
if !path.exists() {
    return;
}
// File could be deleted here
let filetime = FileTime::from_unix_time(*timestamp, 0);
match set_file_mtime(file, filetime) {
```

**Impact:**
- May fail if files are modified between check and use
- Could cause unnecessary errors in concurrent environments

**Recommendation:** Remove the check and handle the error from `set_file_mtime()`:
```rust
let filetime = FileTime::from_unix_time(*timestamp, 0);
match set_file_mtime(file, filetime) {
    Ok(_) => { /* success */ }
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
        // File was deleted, skip silently
        return;
    }
    Err(e) => {
        eprintln!("  ✗ Failed to update mtime for {}: {}", file, e);
    }
}
```

### 4. Incomplete Error Context
**Files:** Multiple
**Severity:** MEDIUM
**Status:** User Experience Issue

Many error messages lack sufficient context for users to understand what went wrong:

**Example from git-fix:337-346:**
```rust
if !output.status.success() {
    eprintln!("✗ ERROR: git {} failed", args.join(" "));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let trimmed = stderr.trim();
    if !trimmed.is_empty() {
        eprintln!("{}", trimmed);
    }
    exit(EXIT_SOFTWARE);
}
```

Missing: What the user was trying to accomplish, current state, recovery options.

**Recommendation:** Add more context to error messages:
```rust
eprintln!("✗ ERROR: Failed to get merge base between {} and {}", head, base);
eprintln!("DETAILS: git merge-base command failed");
eprintln!("NEXT: Verify both commits exist with: git log --oneline");
```

### 5. No Comprehensive Test Suite
**Files:** None found
**Severity:** MEDIUM
**Status:** Missing Tests

No test files were found in the repository.

**Impact:**
- Regressions may be introduced
- Difficult to refactor safely
- Manual testing required for each change

**Recommendation:** Add integration tests for each tool. Consider using:
- Temporary git repositories for testing
- Shell script tests with BATS (Bash Automated Testing System)
- Rust integration tests with git2-rs for validation

### 6. Missing Project Documentation
**Files:** No README.md
**Severity:** MEDIUM
**Status:** Poor Discoverability

The repository lacks documentation explaining:
- What each tool does
- Installation instructions
- Usage examples
- Dependencies (rust-script)

**Recommendation:** Create a comprehensive README.md with:
```markdown
# git-tools

Collection of Git utility scripts for advanced workflows.

## Installation
...

## Tools
- `git-hunk` - Work with individual diff hunks
- `git-conventional-commit` - Create conventional commits
...

## Usage Examples
...
```

---

## Low Priority Issues

### 1. Inconsistent Error Exit Codes
**Files:** Multiple
**Severity:** LOW
**Status:** Minor Inconsistency

Scripts define standard exit codes but usage is inconsistent:

```rust
const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 64;      // Only in some scripts
const EXIT_DATAERR: i32 = 65;
const EXIT_SOFTWARE: i32 = 70;
const EXIT_TEMPFAIL: i32 = 75;   // Only in git-fix
```

Some scripts exit with `EXIT_SOFTWARE` for what should be `EXIT_DATAERR` cases.

**Recommendation:** Standardize exit codes across all scripts and use them consistently.

### 2. Verbose Output Not Standardized
**Files:** Multiple
**Severity:** LOW
**Status:** UX Inconsistency

Some tools have `--verbose` flag, others don't. Verbose output format varies.

**Recommendation:** Add `--verbose` to all tools with consistent formatting:
- Use `→` prefix for actions
- Use `✓` prefix for success
- Use `✗` prefix for errors

### 3. No Version Information
**Files:** All
**Severity:** LOW
**Status:** Missing Feature

Scripts don't include version information or `--version` flag.

**Recommendation:** Add version constant and handle `--version` flag in each script.

### 4. Git Command Output Not Captured Consistently
**Files:** Various
**Severity:** LOW
**Status:** Minor Issue

Some commands use `.status()`, others use `.output()` inconsistently, even when output isn't needed.

**Recommendation:** Use `.status()` when output isn't needed, `.output()` when it is. Be consistent.

---

## Security Analysis

### Authentication & Authorization
✅ **PASS** - Scripts inherit git's authentication mechanisms
✅ **PASS** - No credential storage or handling

### Input Validation
⚠️ **PARTIAL** - Git references are validated, but file paths could be improved
✅ **PASS** - Uses shell-escape crate for command injection prevention

### Command Injection
✅ **PASS** - Proper use of Command API with argument arrays
✅ **PASS** - shell-escape used for shell-quoted output

### Privilege Escalation
✅ **PASS** - No privilege manipulation
✅ **PASS** - Runs with user's git permissions

### Resource Exhaustion
✅ **PASS** - No unbounded loops or recursion
✅ **PASS** - Reasonable memory usage

### Information Disclosure
⚠️ **MINOR** - Error messages could leak filesystem paths
✅ **PASS** - No sensitive data logged

---

## Code Quality Analysis

### Strengths
- ✅ Consistent use of clap for argument parsing
- ✅ Proper error types with thiserror
- ✅ Good user-facing error messages with "NEXT:" hints
- ✅ Consistent exit codes
- ✅ Dry-run support in most tools
- ✅ Clear command structure

### Areas for Improvement
- ❌ Significant code duplication
- ❌ Lack of tests
- ❌ Missing documentation
- ⚠️ Inconsistent error handling patterns
- ⚠️ Some panics from expect()

---

## Recommendations Summary

### Immediate Action Required
1. **Fix git-prune-all** - Remove unused `use regex::Regex;` import (1 line change)
2. **Fix git-reword-commit** - Update hardcoded path to git-new-from

### High Priority
3. Migrate git-fix-branch away from deprecated filter-branch
4. Replace expect() calls with proper error handling

### Medium Priority
5. Add basic integration tests
6. Create README.md with documentation
7. Fix TOCTOU race condition in git-sync-mtime
8. Improve input validation for file paths

### Low Priority
9. Extract common code into shared module
10. Standardize verbose output across all tools
11. Add version information
12. Standardize exit code usage

---

## Conclusion

The git-tools codebase is functional and provides useful Git workflow automation. The code demonstrates good practices in user experience (clear error messages, dry-run support) and basic security (proper command construction). However, the critical bug in git-prune-all requires immediate attention, and the deprecated filter-branch usage should be addressed soon.

The main areas for improvement are:
1. **Reliability** - Fix the compilation bug and remove panics
2. **Maintainability** - Reduce code duplication and add tests
3. **Documentation** - Add README and usage examples

With these improvements, the codebase would be production-ready for broader use.

---

## Detailed File-by-File Analysis

### git-hunk (632 lines)
**Purpose:** Work with git diff hunks - show minimal diffs and stage specific hunks
**Status:** ✅ Generally good
**Issues:**
- Uses expect() for git rev-parse (line 312)
**Strengths:**
- Comprehensive validation
- Good error messages with suggestions
- Proper handling of untracked files

### git-conventional-commit (608 lines)
**Purpose:** Create conventional commits with proper formatting
**Status:** ✅ Generally good
**Issues:**
- Uses expect() for command execution (lines 357, 424)
**Strengths:**
- Excellent validation logic
- Good integration with commitizen
- Comprehensive feature set (fixup, breaking changes, etc.)

### git-fix (1013 lines)
**Purpose:** Automatically apply fixup! and squash! commits
**Status:** ✅ Complex but functional
**Issues:**
- Multiple expect() calls (lines 198, 757, 770, 773, 803, 821, 972)
- Complex state management (could be refactored)
**Strengths:**
- Sophisticated git operation detection
- Handles revert pairs
- Good error recovery messages

### git-push-branch (285 lines)
**Purpose:** Push local branch to remote using push-each
**Status:** ✅ Good
**Issues:**
- Uses expect() (line 101)
- Code duplication
**Strengths:**
- Clear workflow
- Dry-run support

### git-extract-branch (342 lines)
**Purpose:** Extract N commits to a new branch
**Status:** ✅ Good
**Issues:**
- Multiple expect() calls (lines 110, 139, 152, 182)
**Strengths:**
- Good argument handling
- Clear extraction logic

### git-fix-branch (232 lines)
**Purpose:** Fix branch commit data to match author data
**Status:** ⚠️ Uses deprecated tool
**Issues:**
- Uses deprecated git filter-branch (line 140)
- Uses expect() (line 87)
**Recommendation:** High priority to migrate to modern approach

### git-push-each (279 lines)
**Purpose:** Push commits individually for CI
**Status:** ✅ Good
**Issues:**
- Uses expect() (line 87)
**Strengths:**
- Atomic pushes
- Force-with-lease safety
- Set-upstream support

### git-prune-all (233 lines)
**Purpose:** Prune merged branches
**Status:** 🔴 Critical bug
**Issues:**
- **CRITICAL:** Missing regex dependency / unused import (line 10)
- Uses expect() (line 35)
**Fix Required:** Remove unused import immediately

### git-new-from (361 lines)
**Purpose:** Create new commit with same tree
**Status:** ✅ Good
**Issues:**
- Uses expect() (line 137)
- Stdin handling could be more robust
**Strengths:**
- Good metadata preservation
- Flexible message input

### git-reword-commit (253 lines)
**Purpose:** Reword commit using git replace
**Status:** ⚠️ Hardcoded path
**Issues:**
- **HIGH:** Hardcoded path to git-new-from (line 165)
- Uses expect() (line 122)
**Strengths:**
- Clean use of git replace
- Good abstraction

### git-sync-mtime (223 lines)
**Purpose:** Sync file mtimes to git commit dates
**Status:** ⚠️ TOCTOU race
**Issues:**
- TOCTOU race condition (line 142)
- Uses expect() (line 42)
**Strengths:**
- Parallel processing with rayon
- Efficient single git log call

### hooks/post-checkout (127 lines)
**Purpose:** Auto-update default branch on checkout
**Status:** ✅ Good
**Issues:** None significant
**Strengths:**
- Simple and focused
- Good branch detection

---

## Appendix: All expect() Locations

1. git-prune-all:35
2. git-fix-branch:87
3. git-sync-mtime:42
4. git-fix:198
5. git-fix:757
6. git-fix:770
7. git-fix:773
8. git-fix:803
9. git-fix:821
10. git-fix:972
11. git-hunk:312
12. git-extract-branch:110
13. git-extract-branch:139
14. git-extract-branch:152
15. git-extract-branch:182
16. git-reword-commit:122
17. git-reword-commit:166
18. git-conventional-commit:357
19. git-conventional-commit:424
20. git-new-from:137
21. git-push-branch:101
22. git-push-each:87

Total: 22 instances across 11 files
