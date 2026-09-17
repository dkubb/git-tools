//! Real Git integration tests, run with `rust-script --test git-push-up`.
//! Each test owns its repositories and child-process environment. Only gh is faked.

#![expect(
    clippy::unwrap_used,
    reason = "Failed fixture setup must fail the test"
)]

use core::iter::once;
use core::time::Duration;
use std::env;
use std::fs::{self, File, Permissions};
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use tempfile::TempDir;
use wait_timeout::ChildExt as _;

/// Successful process exit.
const SUCCESS: i32 = 0;
/// Rejected workflow.
const FAILURE: i32 = 1;
/// Invalid command line.
const USAGE_ERROR: i32 = 2;

/// Captured child result, retaining exact output for identity assertions.
struct Response {
    /// Exit status; signal termination fails the harness.
    code: i32,
    /// Diagnostic stream.
    stderr: String,
    /// Result stream.
    stdout: String,
}

impl Response {
    /// Require successful fixture commands with useful failure diagnostics.
    fn checked(self) -> Self {
        assert_eq!(self.code, SUCCESS, "{}{}", self.stdout, self.stderr);
        self
    }
}

/// One test's owned repositories, executable overrides, and initial commit IDs.
struct Fixture {
    /// Latest base available on the remote.
    base: String,
    /// Directory for fake gh and the command under test.
    bin: PathBuf,
    /// Optional Git dispatch override used by the fetch race witness.
    exec_path: Option<PathBuf>,
    /// Absolute real Git path, bypassing the race wrapper in fixture commands.
    git_program: String,
    /// Original published feature tip.
    old_feature: String,
    /// Bare remote repository.
    remote: PathBuf,
    /// Selected client checkout.
    repo: PathBuf,
    /// Lifetime owner; removes every fixture file on drop, including failures.
    root: TempDir,
}

impl Fixture {
    /// Assert the original destination has not changed.
    fn assert_not_pushed(&self) {
        assert_eq!(self.rev_at("feature", &self.remote), self.old_feature);
    }

    /// Run an arbitrary successful fixture command.
    fn command(&self, program: &str, args: &[&str], cwd: &Path) -> Response {
        self.run(program, args, cwd).checked()
    }

    /// Add a commit to the client.
    fn commit(&self, filename: &str, contents: &str, subject: &str) {
        self.commit_at(filename, contents, subject, &self.repo);
    }

    /// Add a commit to a specific fixture checkout.
    fn commit_at(&self, filename: &str, contents: &str, subject: &str, repo: &Path) {
        fs::write(repo.join(filename), contents).unwrap();
        self.git_at(&["add", filename], repo);
        self.git_at(&["commit", "-m", subject], repo);
    }

    /// Supply deterministic GitHub discovery output.
    fn fake_gh(&self, body: &str) {
        let executable = self.bin.join("gh");
        fs::write(&executable, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(executable, Permissions::from_mode(0o755)).unwrap();
    }

    /// Run a successful Git command in the selected checkout.
    fn git(&self, args: &[&str]) -> Response {
        self.git_at(args, &self.repo)
    }

    /// Run a successful Git command in a chosen directory.
    fn git_at(&self, args: &[&str], cwd: &Path) -> Response {
        self.git_result_at(args, cwd).checked()
    }

    /// Inspect a Git command that may reject an operation.
    fn git_result(&self, args: &[&str]) -> Response {
        self.git_result_at(args, &self.repo)
    }

    /// Inspect a Git command in a chosen directory.
    fn git_result_at(&self, args: &[&str], cwd: &Path) -> Response {
        self.run(&self.git_program, args, cwd)
    }

    /// Configure deterministic authorship without reading the user's config.
    fn identity(&self, repo: &Path) {
        self.git_at(&["config", "user.name", "Test"], repo);
        self.git_at(&["config", "user.email", "test@example.invalid"], repo);
    }

    /// Exercise the command through Git's global -C dispatch.
    fn invoke(&self, args: &[&str]) -> Response {
        self.invoke_result(args).checked()
    }

    /// Exercise a command that may reject publication.
    fn invoke_result(&self, args: &[&str]) -> Response {
        let mut arguments = vec!["-C", path(&self.repo), "push-up"];
        arguments.extend_from_slice(args);
        self.git_result_at(&arguments, self.root.path())
    }

    /// Create a feature branch behind a newly advanced base on a private remote.
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let remote = root.path().join("remote.git");
        let repo = root.path().join("repo");
        let bin = root.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        symlink(binary(), bin.join("git-push-up")).unwrap();
        let git_program = env::split_paths(&env::var_os("PATH").unwrap())
            .map(|directory| directory.join("git"))
            .find(|candidate| candidate.is_file())
            .unwrap();
        let mut fixture = Self {
            base: String::new(),
            bin,
            exec_path: None,
            git_program: path(&git_program).to_owned(),
            old_feature: String::new(),
            remote,
            repo,
            root,
        };
        fixture.git_at(
            &[
                "init",
                "--bare",
                "--initial-branch=main",
                path(&fixture.remote),
            ],
            fixture.root.path(),
        );
        fixture.git_at(
            &["clone", path(&fixture.remote), path(&fixture.repo)],
            fixture.root.path(),
        );
        fixture.identity(&fixture.repo);
        fixture.commit("initial.txt", "initial", "Add initial file");
        fixture.git(&["push", "origin", "main"]);
        fixture.git(&["switch", "-c", "feature"]);
        fixture.commit("feature.txt", "feature", "Add feature");
        fixture.git(&["push", "-u", "origin", "feature"]);
        fixture.old_feature = fixture.rev("feature");
        fixture.git(&["switch", "main"]);
        fixture.commit("base.txt", "base", "Add base change");
        fixture.git(&["push", "origin", "main"]);
        fixture.base = fixture.rev("main");
        fixture.git(&["switch", "feature"]);
        fixture
    }

    /// Read a fixture object ID from the client.
    fn rev(&self, reference: &str) -> String {
        self.rev_at(reference, &self.repo)
    }

    /// Read a fixture object ID without changing its spelling.
    fn rev_at(&self, reference: &str, repo: &Path) -> String {
        let response = self.git_at(&["rev-parse", reference], repo);
        response.stdout.strip_suffix('\n').unwrap().to_owned()
    }

    /// Bound subprocess runtime and isolate Git configuration per child.
    fn run(&self, program: &str, args: &[&str], cwd: &Path) -> Response {
        let mut command = Command::new(program);
        command.args(args).current_dir(cwd);
        for (key, _) in env::vars_os() {
            if key.as_encoded_bytes().starts_with(b"GIT_") {
                command.env_remove(key);
            }
        }
        let inherited = env::var_os("PATH").unwrap();
        let search =
            env::join_paths(once(self.bin.clone()).chain(env::split_paths(&inherited))).unwrap();
        command
            .env("PATH", search)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true");
        if let Some(directory) = self.exec_path.as_ref() {
            command.env("GIT_EXEC_PATH", directory);
        }
        capture(&mut command)
    }
}

/// Build the production executable once in a separate target to avoid Cargo lock recursion.
fn binary() -> &'static Path {
    static EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();
    EXECUTABLE.get_or_init(|| {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target = manifest.join("target/integration");
        capture(Command::new("cargo").args([
            "build",
            "--quiet",
            "--manifest-path",
            path(&manifest.join("Cargo.toml")),
            "--target-dir",
            path(&target),
        ]))
        .checked();
        let executable = target.join("debug/git-push-up");
        fs::copy(
            target.join("debug").join(env!("CARGO_PKG_NAME")),
            &executable,
        )
        .unwrap();
        executable
    })
}

/// Capture to files so verbose children cannot block on full pipe buffers.
fn capture(command: &mut Command) -> Response {
    let directory = tempfile::tempdir().unwrap();
    let stdout = directory.path().join("stdout");
    let stderr = directory.path().join("stderr");
    command
        .stdout(Stdio::from(File::create(&stdout).unwrap()))
        .stderr(Stdio::from(File::create(&stderr).unwrap()));
    let mut child = command.spawn().unwrap();
    let status = child.wait_timeout(Duration::from_mins(2)).unwrap();
    if status.is_none() {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    assert!(status.is_some(), "command timed out: {command:?}");
    Response {
        code: status.unwrap().code().unwrap(),
        stderr: fs::read_to_string(stderr).unwrap(),
        stdout: fs::read_to_string(stdout).unwrap(),
    }
}

/// Borrow paths as command arguments without lossy conversion.
fn path(value: &Path) -> &str {
    value.to_str().unwrap()
}
/// Quote one argument for the tiny executable fixture hooks.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
/// Encode only fixture-owned command arguments into a shell hook.
fn shell(args: &[&str]) -> String {
    args.iter()
        .map(|value| quote(value))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Branch in other worktree is not moved.
#[test]
fn branch_in_other_worktree_is_not_moved() {
    let fixture = Fixture::new();
    fixture.git(&["branch", "sibling"]);
    fixture.git(&[
        "worktree",
        "add",
        path(&fixture.root.path().join("sibling")),
        "sibling",
    ]);
    fixture.invoke(&["--base", "main"]);
    assert_eq!(fixture.rev("sibling"), fixture.old_feature);
    assert_ne!(fixture.rev("feature"), fixture.old_feature);
}

/// Configured push refspec maps explicit source.
#[test]
fn configured_push_refspec_maps_explicit_source() {
    let fixture = Fixture::new();
    fixture.git(&["branch", "temporary", "feature"]);
    fixture.git(&[
        "config",
        "remote.origin.push",
        "refs/heads/temporary:refs/heads/feature",
    ]);
    fixture.invoke(&["--replace", "--base", "main", "temporary"]);
    assert_eq!(
        fixture.rev("temporary"),
        fixture.rev_at("feature", &fixture.remote)
    );
}

/// Configured push refspec selects branch.
#[test]
fn configured_push_refspec_selects_branch() {
    let fixture = Fixture::new();
    fixture.git(&["branch", "temporary", "feature"]);
    fixture.git(&[
        "config",
        "remote.origin.push",
        "refs/heads/temporary:refs/heads/feature",
    ]);
    fixture.git(&["config", "push.default", "nothing"]);
    fixture.invoke(&["--replace", "--base", "main"]);
    assert_eq!(fixture.rev("temporary^"), fixture.base);
    assert_eq!(
        fixture.rev("temporary"),
        fixture.rev_at("feature", &fixture.remote)
    );
    assert_eq!(fixture.rev("feature"), fixture.old_feature);
}

/// Conflict does not push.
#[test]
fn conflict_does_not_push() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "main"]);
    fixture.commit("feature.txt", "conflict", "Add conflicting base change");
    fixture.git(&["push", "origin", "main"]);
    fixture.git(&["switch", "feature"]);
    let result = fixture.invoke_result(&["--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    assert_eq!(
        fixture
            .git_result(&["rev-parse", "--verify", "REBASE_HEAD"])
            .code,
        SUCCESS
    );
    fixture.assert_not_pushed();
}

/// Current and automatic setup choose same name.
#[test]
fn current_and_automatic_setup_choose_same_name() {
    let fixture = Fixture::new();
    fixture.git(&["branch", "--unset-upstream"]);
    fixture.git(&["config", "push.default", "current"]);
    let mut plan = fixture.invoke(&["--dry-run", "--base", "main"]);
    assert!(
        plan.stdout.contains("destination: origin/feature"),
        "{}",
        plan.stdout
    );
    fixture.git(&["config", "push.default", "simple"]);
    fixture.git(&["config", "push.autoSetupRemote", "true"]);
    plan = fixture.invoke(&["--dry-run", "--base", "main"]);
    assert!(
        plan.stdout.contains("destination: origin/feature"),
        "{}",
        plan.stdout
    );
}

/// Current branch rebases and pushes.
#[test]
fn current_branch_rebases_and_pushes() {
    let fixture = Fixture::new();
    fixture.git(&[
        "update-ref",
        "refs/remotes/origin/main",
        &fixture.rev("feature^"),
    ]);
    fixture.invoke(&["--base", "main"]);
    assert_eq!(fixture.rev("feature^"), fixture.base);
    assert_eq!(fixture.rev("origin/main"), fixture.base);
    assert_eq!(
        fixture.rev("feature"),
        fixture.rev_at("feature", &fixture.remote)
    );
    assert_ne!(fixture.rev("feature"), fixture.old_feature);
}

/// Detached head refspec is rejected.
#[test]
fn detached_head_refspec_is_rejected() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "--detach"]);
    let result = fixture.invoke_result(&["--base", "main", "HEAD:feature"]);
    assert_ne!(result.code, SUCCESS);
    assert!(
        result.stderr.contains("pass a branch explicitly"),
        "{}",
        result.stderr
    );
    fixture.assert_not_pushed();
}

/// Detached head requires a selected branch.
#[test]
fn detached_head_requires_a_selected_branch() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "--detach"]);
    let result = fixture.invoke_result(&["--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    assert!(
        result.stderr.contains("pass a branch explicitly"),
        "{}",
        result.stderr
    );
    fixture.assert_not_pushed();
}

/// Dirty worktree fails before fetch.
#[test]
fn dirty_worktree_fails_before_fetch() {
    let fixture = Fixture::new();
    fs::write(fixture.repo.join("untracked"), "do not lose").unwrap();
    fixture.git(&["update-ref", "-d", "refs/remotes/origin/main"]);
    let result = fixture.invoke_result(&["--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    assert!(
        result.stderr.contains("clean worktree"),
        "{}",
        result.stderr
    );
    assert_ne!(
        fixture
            .git_result(&["show-ref", "--verify", "refs/remotes/origin/main"])
            .code,
        SUCCESS
    );
    fixture.assert_not_pushed();
}

/// Does not follow tags from user configuration.
#[test]
fn does_not_follow_tags_from_user_configuration() {
    let fixture = Fixture::new();
    fixture.git(&["config", "push.followTags", "true"]);
    fixture.git(&["tag", "-a", "base-tag", "origin/main", "-m", "Base tag"]);
    fixture.invoke(&["--base", "main"]);
    assert_eq!(fixture.git_at(&["tag"], &fixture.remote).stdout, "");
}

/// Dry run does not fetch or rewrite.
#[test]
fn dry_run_does_not_fetch_or_rewrite() {
    let fixture = Fixture::new();
    fixture.git(&["update-ref", "-d", "refs/remotes/origin/main"]);
    let before = fixture.git(&["show-ref"]).stdout;
    let result = fixture.invoke(&["--base", "main", "--dry-run"]);
    assert!(
        result
            .stdout
            .contains("--force-with-lease --force-if-includes"),
        "{}",
        result.stdout
    );
    assert_eq!(fixture.git(&["show-ref"]).stdout, before);
    fixture.assert_not_pushed();
}

/// Empty pr list fails before mutation.
#[test]
fn empty_pr_list_fails_before_mutation() {
    let fixture = Fixture::new();
    fixture.fake_gh("exit 0");
    let before = fixture.git(&["show-ref"]).stdout;
    let result = fixture.invoke_result(&[]);
    assert_eq!(result.code, FAILURE);
    assert!(
        result.stderr.contains("expected exactly one open PR"),
        "{}",
        result.stderr
    );
    assert_eq!(fixture.git(&["show-ref"]).stdout, before);
    fixture.assert_not_pushed();
}

/// Explicit base bypasses gh.
#[test]
fn explicit_base_bypasses_gh() {
    let fixture = Fixture::new();
    fixture.fake_gh("exit 99");
    fixture.invoke(&["--base", "main"]);
}

/// Explicit destination overrides configured mapping.
#[test]
fn explicit_destination_overrides_configured_mapping() {
    let fixture = Fixture::new();
    fixture.git(&["config", "remote.origin.push", "feature:wrong"]);
    fixture.invoke(&["--base", "main", "feature:feature"]);
    assert_eq!(
        fixture.rev("feature"),
        fixture.rev_at("feature", &fixture.remote)
    );
}

/// Explicit destination uses its pr.
#[test]
fn explicit_destination_uses_its_pr() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "-c", "temporary"]);
    fixture.fake_gh("test \"$6\" = feature || exit 3; printf \"main\\n\"");
    fixture.invoke(&["temporary:feature"]);
    assert_eq!(fixture.rev("temporary^"), fixture.base);
    assert_eq!(
        fixture.rev("temporary"),
        fixture.rev_at("feature", &fixture.remote)
    );
    assert_ne!(
        fixture
            .git_result_at(
                &["show-ref", "--verify", "refs/heads/temporary"],
                &fixture.remote
            )
            .code,
        SUCCESS
    );
}

/// Explicit source uses checked out branch remote.
#[test]
fn explicit_source_uses_checked_out_branch_remote() {
    let fixture = Fixture::new();
    fixture.git(&["remote", "add", "publish", path(&fixture.remote)]);
    fixture.git(&["config", "branch.main.pushRemote", "publish"]);
    fixture.git(&["config", "branch.feature.pushRemote", "missing"]);
    fixture.git(&["switch", "main"]);
    let plan = fixture.invoke(&["--dry-run", "--base", "main", "feature:feature"]);
    assert!(
        plan.stdout.contains("destination: publish/feature"),
        "{}",
        plan.stdout
    );
}

/// Explicit source works with detached head.
#[test]
fn explicit_source_works_with_detached_head() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "--detach"]);
    fixture.invoke(&["--replace", "--base", "main", "feature:feature"]);
    assert_eq!(
        fixture.rev("feature"),
        fixture.rev_at("feature", &fixture.remote)
    );
}

/// Failed fetch does not rebase.
#[test]
fn failed_fetch_does_not_rebase() {
    let fixture = Fixture::new();
    let result = fixture.invoke_result(&["--base", "missing"]);
    assert_ne!(result.code, SUCCESS);
    assert_eq!(fixture.rev("feature"), fixture.old_feature);
    fixture.assert_not_pushed();
}

/// Fetch result pins base and destination before background fetch.
#[test]
fn fetch_result_pins_base_and_destination_before_background_fetch() {
    let mut fixture = Fixture::new();
    let other = fixture.root.path().join("other");
    fixture.git(&[
        "clone",
        "--branch",
        "feature",
        path(&fixture.remote),
        path(&other),
    ]);
    fixture.identity(&other);
    fixture.commit_at(
        "other.txt",
        "other",
        "Add concurrent destination change",
        &other,
    );
    let remote_tip = fixture.rev_at("feature", &other);
    fixture.git_at(&["switch", "main"], &other);
    fixture.commit_at("new-base.txt", "new", "Advance base concurrently", &other);
    let new_base = fixture.rev_at("main", &other);
    let wrapper = fixture.bin.join("git");
    let git = quote(&fixture.git_program);
    let push = shell(&[
        &fixture.git_program,
        "-C",
        path(&other),
        "push",
        "origin",
        "feature",
        "main",
    ]);
    let fetch = shell(&[
        &fixture.git_program,
        "fetch",
        "origin",
        "+refs/heads/feature:refs/remotes/origin/feature",
        "+refs/heads/main:refs/remotes/origin/main",
    ]);
    fs::write(&wrapper, format!("#!/bin/sh\nset -eu\nif test \"$1\" = fetch; then\n{git} \"$@\"\n{push} >&2\n{fetch} >&2\nexit 0\nfi\nexec {git} \"$@\"\n")).unwrap();
    fs::set_permissions(&wrapper, Permissions::from_mode(0o755)).unwrap();
    fixture.exec_path = Some(fixture.bin.clone());
    let result = fixture.invoke_result(&["--replace", "--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    assert!(result.stderr.contains("stale info"), "{}", result.stderr);
    assert_eq!(fixture.rev_at("feature", &fixture.remote), remote_tip);
    assert_eq!(fixture.rev("origin/feature"), remote_tip);
    assert_eq!(fixture.rev("origin/main"), new_base);
    assert_eq!(fixture.rev("feature^"), fixture.base);
}

/// Force if includes rejects background fetch after rebase.
#[test]
fn force_if_includes_rejects_background_fetch_after_rebase() {
    let fixture = Fixture::new();
    let other = fixture.root.path().join("other");
    fixture.git(&[
        "clone",
        "--branch",
        "feature",
        path(&fixture.remote),
        path(&other),
    ]);
    fixture.identity(&other);
    fixture.commit_at("other.txt", "other", "Add concurrent change", &other);
    let remote_tip = fixture.rev_at("feature", &other);
    let hook = fixture.repo.join(".git").join("hooks").join("post-rewrite");
    let push = shell(&[
        &fixture.git_program,
        "-C",
        path(&other),
        "push",
        "origin",
        "feature",
    ]);
    let fetch = shell(&[
        &fixture.git_program,
        "fetch",
        "origin",
        "+refs/heads/feature:refs/remotes/origin/feature",
    ]);
    fs::write(&hook, format!("#!/bin/sh\nset -eu\n{push}\n{fetch}\n")).unwrap();
    fs::set_permissions(&hook, Permissions::from_mode(0o755)).unwrap();
    let result = fixture.invoke_result(&["--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    assert_eq!(fixture.rev_at("feature", &fixture.remote), remote_tip);
    assert_eq!(fixture.rev("origin/feature"), remote_tip);
    assert!(result.stderr.contains("rejected"), "{}", result.stderr);
}

/// Forwarded help works outside repository.
#[test]
fn forwarded_help_works_outside_repository() {
    let fixture = Fixture::new();
    let direct = fixture.command(path(binary()), &["--help"], fixture.root.path());
    let forwarded = fixture.git_at(&["push-up", "--", "--help"], fixture.root.path());
    assert_eq!(forwarded.stdout, direct.stdout);
    assert_eq!(forwarded.stderr, "");
    fixture.assert_not_pushed();
}

/// Gh resolves selected branch.
#[test]
fn gh_resolves_selected_branch() {
    let fixture = Fixture::new();
    fixture.fake_gh(&format!("test \"$*\" = \"pr list --repo {} --head feature --state open --limit 2 --json baseRefName --jq .[].baseRefName\" || exit 3\nprintf \"main\\n\"", path(&fixture.remote)));
    fixture.git(&["switch", "main"]);
    fixture.invoke(&["feature"]);
    assert_eq!(fixture.rev("feature^"), fixture.base);
}

/// Head source resolves current branch.
#[test]
fn head_source_resolves_current_branch() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "-c", "temporary"]);
    fixture.invoke(&["--base", "main", "HEAD:feature"]);
    assert_eq!(
        fixture.rev("temporary"),
        fixture.rev_at("feature", &fixture.remote)
    );
    assert_eq!(fixture.rev("temporary^"), fixture.base);
}

/// Invalid refspecs fail before mutation.
#[test]
fn invalid_refspecs_fail_before_mutation() {
    let fixture = Fixture::new();
    let before = fixture.git(&["show-ref"]).stdout;
    for refspec in [
        ":feature",
        "feature:",
        ":",
        "feature:other:third",
        "*:feature",
        "+feature:feature",
        "HEAD~1:feature",
        "feature:main",
    ] {
        let result = fixture.invoke_result(&["--base", "main", refspec]);
        assert_ne!(result.code, SUCCESS);
        assert_eq!(fixture.git(&["show-ref"]).stdout, before);
        fixture.assert_not_pushed();
    }
}

/// Matching and nothing require explicit source.
#[test]
fn matching_and_nothing_require_explicit_source() {
    let fixture = Fixture::new();
    for policy in ["matching", "nothing"] {
        fixture.git(&["config", "push.default", policy]);
        let result = fixture.invoke_result(&["--base", "main"]);
        assert_ne!(result.code, SUCCESS);
        fixture.assert_not_pushed();
        let plan = fixture.invoke(&["--dry-run", "--base", "main", "feature:feature"]);
        assert!(
            plan.stdout.contains("destination: origin/feature"),
            "{}",
            plan.stdout
        );
    }
}

/// Missing pr fails before mutation.
#[test]
fn missing_pr_fails_before_mutation() {
    let fixture = Fixture::new();
    fixture.fake_gh("echo 'no PR found' >&2; exit 1");
    let before = fixture.git(&["show-ref"]).stdout;
    let result = fixture.invoke_result(&[]);
    assert_ne!(result.code, SUCCESS);
    assert!(
        result.stderr.contains("pass --base explicitly"),
        "{}",
        result.stderr
    );
    assert_eq!(fixture.git(&["show-ref"]).stdout, before);
    fixture.assert_not_pushed();
}

/// Multiple prs fail before mutation.
#[test]
fn multiple_prs_fail_before_mutation() {
    let fixture = Fixture::new();
    fixture.fake_gh("printf \"main\\nother\\n\"");
    let before = fixture.git(&["show-ref"]).stdout;
    let result = fixture.invoke_result(&[]);
    assert_eq!(result.code, FAILURE);
    assert!(
        result.stderr.contains("expected exactly one open PR"),
        "{}",
        result.stderr
    );
    assert_eq!(fixture.git(&["show-ref"]).stdout, before);
    fixture.assert_not_pushed();
}

/// Multiple push mappings are rejected.
#[test]
fn multiple_push_mappings_are_rejected() {
    let fixture = Fixture::new();
    fixture.git(&["config", "--add", "remote.origin.push", "feature:feature"]);
    fixture.git(&["config", "--add", "remote.origin.push", "main:main"]);
    let result = fixture.invoke_result(&["--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    fixture.assert_not_pushed();
}

/// Multiple push urls are rejected.
#[test]
fn multiple_push_urls_are_rejected() {
    let fixture = Fixture::new();
    fixture.git(&[
        "config",
        "--add",
        "remote.origin.pushurl",
        path(&fixture.remote),
    ]);
    fixture.git(&[
        "config",
        "--add",
        "remote.origin.pushurl",
        path(&fixture.remote),
    ]);
    let result = fixture.invoke_result(&["--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    fixture.assert_not_pushed();
}

/// Numeric branch uses head filter.
#[test]
fn numeric_branch_uses_head_filter() {
    let fixture = Fixture::new();
    fixture.git(&["branch", "123", "refs/heads/feature"]);
    fixture.git(&["push", "origin", "refs/heads/123:refs/heads/123"]);
    fixture.fake_gh(&format!("test \"$*\" = \"pr list --repo {} --head 123 --state open --limit 2 --json baseRefName --jq .[].baseRefName\" || exit 3\nprintf \"main\\n\"", path(&fixture.remote)));
    fixture.invoke(&["123"]);
    assert_eq!(fixture.rev("refs/heads/123^"), fixture.base);
    assert_eq!(
        fixture.rev("refs/heads/123"),
        fixture.rev_at("refs/heads/123", &fixture.remote)
    );
    fixture.assert_not_pushed();
}

/// Push remote precedence.
#[test]
fn push_remote_precedence() {
    let fixture = Fixture::new();
    fixture.git(&["remote", "add", "global", path(&fixture.remote)]);
    fixture.git(&["remote", "add", "branch", path(&fixture.remote)]);
    fixture.git(&["config", "remote.pushDefault", "global"]);
    let mut plan = fixture.invoke(&["--dry-run", "--base", "main"]);
    assert!(
        plan.stdout.contains("destination: global/feature"),
        "{}",
        plan.stdout
    );
    fixture.git(&["config", "branch.feature.pushRemote", "branch"]);
    plan = fixture.invoke(&["--dry-run", "--base", "main"]);
    assert!(
        plan.stdout.contains("destination: branch/feature"),
        "{}",
        plan.stdout
    );
}

/// Rejects merge commits without flattening them.
#[test]
fn rejects_merge_commits_without_flattening_them() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "-c", "side", "feature^"]);
    fixture.commit("side.txt", "side", "Add side change");
    fixture.git(&["switch", "feature"]);
    fixture.git(&["merge", "--no-ff", "side", "-m", "Merge side"]);
    let before = fixture.rev("feature");
    let result = fixture.invoke_result(&["--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    assert!(
        result.stderr.contains("linear branch is required"),
        "{}",
        result.stderr
    );
    assert_eq!(fixture.rev("feature"), before);
    fixture.assert_not_pushed();
}

/// Rejects same base and source.
#[test]
fn rejects_same_base_and_source() {
    let fixture = Fixture::new();
    let result = fixture.invoke_result(&["--base", "feature"]);
    assert_ne!(result.code, SUCCESS);
    assert!(result.stderr.contains("must differ"), "{}", result.stderr);
    fixture.assert_not_pushed();
}

/// Rejects unknown options.
#[test]
fn rejects_unknown_options() {
    let fixture = Fixture::new();
    let result = fixture.invoke_result(&["--repo", path(&fixture.repo)]);
    assert_eq!(result.code, USAGE_ERROR);
    assert!(
        result.stderr.contains("unexpected argument"),
        "{}",
        result.stderr
    );
    fixture.assert_not_pushed();
}

/// Remote changes must be integrated before rebase.
#[test]
fn remote_changes_must_be_integrated_before_rebase() {
    let fixture = Fixture::new();
    let other = fixture.root.path().join("other");
    fixture.git(&[
        "clone",
        "--branch",
        "feature",
        path(&fixture.remote),
        path(&other),
    ]);
    fixture.identity(&other);
    fixture.commit_at("other.txt", "other", "Add remote change", &other);
    fixture.git_at(&["push", "origin", "feature"], &other);
    let remote_tip = fixture.rev_at("feature", &fixture.remote);
    let result = fixture.invoke_result(&["--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    assert!(
        result.stderr.contains("integrate it first"),
        "{}",
        result.stderr
    );
    assert_eq!(fixture.rev("feature"), fixture.old_feature);
    assert_eq!(fixture.rev_at("feature", &fixture.remote), remote_tip);
}

/// Renamed remote is used for fetch and push.
#[test]
fn renamed_remote_is_used_for_fetch_and_push() {
    let fixture = Fixture::new();
    fixture.git(&["remote", "rename", "origin", "publish"]);
    fixture.git(&[
        "update-ref",
        "refs/remotes/publish/main",
        &fixture.rev("feature^"),
    ]);
    fixture.invoke(&["--base", "main"]);
    assert_eq!(fixture.rev("publish/main"), fixture.base);
    assert_eq!(fixture.rev("feature^"), fixture.base);
    assert_eq!(
        fixture.rev("feature"),
        fixture.rev_at("feature", &fixture.remote)
    );
}

/// Replacement conflict does not publish.
#[test]
fn replacement_conflict_does_not_publish() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "main"]);
    fixture.commit("feature.txt", "conflict", "Add conflict");
    fixture.git(&["push", "origin", "main"]);
    fixture.git(&["switch", "feature"]);
    let result = fixture.invoke_result(&["--replace", "--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    assert_eq!(
        fixture
            .git_result(&["rev-parse", "--verify", "REBASE_HEAD"])
            .code,
        SUCCESS
    );
    fixture.assert_not_pushed();
}

/// Replacement dry run explains policy without fetch.
#[test]
fn replacement_dry_run_explains_policy_without_fetch() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "-c", "temporary"]);
    fixture.git(&["update-ref", "-d", "refs/remotes/origin/main"]);
    let before = fixture.git(&["show-ref"]).stdout;
    let result = fixture.invoke(&[
        "--replace",
        "--dry-run",
        "--base",
        "main",
        "temporary:feature",
    ]);
    for text in [
        "Mode: replacement",
        "Source: temporary",
        "destination: origin/feature",
        "base: origin/main",
        "PR lookup uses destination feature",
        "Skip destination ancestry",
        "do not use --force-if-includes",
        "--no-update-refs",
        "--force-with-lease=refs/heads/feature:<fetched-destination-sha>",
        "unknown until execution",
    ] {
        assert!(result.stdout.contains(text), "{}", result.stdout);
    }
    assert_eq!(fixture.git(&["show-ref"]).stdout, before);
    fixture.assert_not_pushed();
}

/// Replacement from fresh clone preserves other refs.
#[test]
fn replacement_from_fresh_clone_preserves_other_refs() {
    let mut fixture = Fixture::new();
    let fresh = fixture.root.path().join("fresh");
    fixture.git(&[
        "clone",
        "--branch",
        "feature",
        path(&fixture.remote),
        path(&fresh),
    ]);
    fixture.repo = fresh.clone();
    fixture.identity(&fresh);
    assert_eq!(
        fixture.git(&["reflog", "show", "origin/feature"]).stdout,
        ""
    );
    fixture.git(&["switch", "-c", "temporary/rewrite"]);
    fixture.git(&["commit", "--amend", "-m", "Reconstruct feature"]);
    let reconstructed_tree = fixture.rev("HEAD^{tree}");
    assert_eq!(reconstructed_tree, fixture.rev("origin/feature^{tree}"));
    fixture.git(&["branch", "sibling"]);
    let sibling = fixture.rev("sibling");
    fixture.git(&["config", "rebase.updateRefs", "true"]);
    fixture.git(&["config", "push.followTags", "true"]);
    fixture.git(&["tag", "-a", "base-tag", "origin/main", "-m", "Base tag"]);
    fixture.fake_gh("test \"$6\" = feature || exit 3; printf \"main\\n\"");
    let before = fixture
        .git_at(
            &["for-each-ref", "--format=%(refname) %(objectname)"],
            &fixture.remote,
        )
        .stdout;
    fixture.invoke(&["--replace", "HEAD:feature"]);
    let after = fixture
        .git_at(
            &["for-each-ref", "--format=%(refname) %(objectname)"],
            &fixture.remote,
        )
        .stdout;
    assert_eq!(
        after,
        before.replace(&fixture.old_feature, &fixture.rev("temporary/rewrite"))
    );
    assert_eq!(fixture.rev("temporary/rewrite^"), fixture.base);
    assert_eq!(fixture.rev("feature"), fixture.old_feature);
    assert_eq!(fixture.rev("sibling"), sibling);
    assert_eq!(
        fixture.rev_at("feature", &fixture.remote),
        fixture.rev("temporary/rewrite")
    );
}

/// Replacement lease rejects concurrent change even after fetch.
#[test]
fn replacement_lease_rejects_concurrent_change_even_after_fetch() {
    let fixture = Fixture::new();
    let other = fixture.root.path().join("other");
    fixture.git(&[
        "clone",
        "--branch",
        "feature",
        path(&fixture.remote),
        path(&other),
    ]);
    fixture.identity(&other);
    fixture.commit_at("other.txt", "other", "Add concurrent change", &other);
    let remote_tip = fixture.rev_at("feature", &other);
    fixture.git(&["switch", "-c", "temporary"]);
    fixture.git(&["commit", "--amend", "-m", "Reconstruct feature"]);
    let hook = fixture.repo.join(".git").join("hooks").join("post-rewrite");
    let push = shell(&[
        &fixture.git_program,
        "-C",
        path(&other),
        "push",
        "origin",
        "feature",
    ]);
    let fetch = shell(&[
        &fixture.git_program,
        "fetch",
        "origin",
        "+refs/heads/feature:refs/remotes/origin/feature",
    ]);
    fs::write(&hook, format!("#!/bin/sh\nset -eu\n{push}\n{fetch}\n")).unwrap();
    fs::set_permissions(&hook, Permissions::from_mode(0o755)).unwrap();
    let result = fixture.invoke_result(&["--replace", "--base", "main", "temporary:feature"]);
    assert_ne!(result.code, SUCCESS);
    assert!(result.stderr.contains("stale info"), "{}", result.stderr);
    assert_eq!(fixture.rev_at("feature", &fixture.remote), remote_tip);
    assert_eq!(fixture.rev("origin/feature"), remote_tip);
    assert_eq!(fixture.rev("temporary^"), fixture.base);
}

/// Replacement publishes pre rewritten same name.
#[test]
fn replacement_publishes_pre_rewritten_same_name() {
    let fixture = Fixture::new();
    fixture.git(&["commit", "--amend", "-m", "Reconstruct feature"]);
    let rewritten = fixture.rev("feature");
    let denied = fixture.invoke_result(&["--base", "main"]);
    assert!(
        denied.stderr.contains("integrate it first"),
        "{}",
        denied.stderr
    );
    assert_eq!(fixture.rev("feature"), rewritten);
    fixture.assert_not_pushed();
    fixture.invoke(&["--replace", "--base", "main"]);
    assert_eq!(fixture.rev("feature^"), fixture.base);
    assert_eq!(
        fixture.rev("feature"),
        fixture.rev_at("feature", &fixture.remote)
    );
}

/// Replacement requires clean worktree.
#[test]
fn replacement_requires_clean_worktree() {
    let fixture = Fixture::new();
    fs::write(fixture.repo.join("untracked"), "preserve").unwrap();
    let result = fixture.invoke_result(&["--replace", "--base", "main"]);
    assert!(
        result.stderr.contains("clean worktree"),
        "{}",
        result.stderr
    );
    fixture.assert_not_pushed();
}

/// Selected branch updates local refs only.
#[test]
fn selected_branch_updates_local_refs_only() {
    let fixture = Fixture::new();
    fixture.git(&["branch", "sibling"]);
    fixture.git(&["push", "origin", "sibling"]);
    fixture.git(&["switch", "main"]);
    fixture.invoke(&["--base", "main", "feature"]);
    assert_eq!(fixture.rev("sibling"), fixture.rev("feature"));
    assert_eq!(
        fixture.rev_at("sibling", &fixture.remote),
        fixture.old_feature
    );
    assert_eq!(
        fixture.git(&["branch", "--show-current"]).stdout.trim(),
        "feature"
    );
}

/// Silent query failure reports status.
#[test]
fn silent_query_failure_reports_status() {
    let fixture = Fixture::new();
    fixture.fake_gh("exit 7");
    let before = fixture.git(&["show-ref"]).stdout;
    let result = fixture.invoke_result(&[]);
    assert_eq!(result.code, FAILURE);
    assert_eq!(result.stdout, "");
    assert_eq!(
        result.stderr,
        format!(
            "error: could not resolve the PR base for feature; pass --base explicitly: gh pr list --repo {} --head feature --state open --limit 2 --json baseRefName --jq .[].baseRefName failed: exit status: 7; stderr: \"\"\n",
            path(&fixture.remote)
        )
    );
    assert_eq!(fixture.git(&["show-ref"]).stdout, before);
    fixture.assert_not_pushed();
}

/// Simple rejects differently named upstream.
#[test]
fn simple_rejects_differently_named_upstream() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "-c", "local", "feature"]);
    fixture.git(&["branch", "--set-upstream-to", "origin/feature"]);
    let result = fixture.invoke_result(&["--base", "main"]);
    assert!(
        result.stderr.contains("simple mode requires matching"),
        "{}",
        result.stderr
    );
    fixture.assert_not_pushed();
}

/// Simple requires upstream but explicit source does not.
#[test]
fn simple_requires_upstream_but_explicit_source_does_not() {
    let fixture = Fixture::new();
    fixture.git(&["branch", "--unset-upstream"]);
    let result = fixture.invoke_result(&["--base", "main"]);
    assert!(result.stderr.contains("no upstream"), "{}", result.stderr);
    fixture.invoke(&["--base", "main", "feature"]);
    assert_eq!(
        fixture.rev("feature"),
        fixture.rev_at("feature", &fixture.remote)
    );
}

/// Simple requires upstream with sole non origin remote.
#[test]
fn simple_requires_upstream_with_sole_non_origin_remote() {
    let fixture = Fixture::new();
    fixture.git(&["branch", "--unset-upstream"]);
    fixture.git(&["remote", "rename", "origin", "publish"]);
    let expected = fixture.git_result(&["push", "--dry-run"]);
    assert_ne!(expected.code, SUCCESS);
    let result = fixture.invoke_result(&["--base", "main"]);
    assert_ne!(result.code, SUCCESS);
    assert!(result.stderr.contains("no upstream"), "{}", result.stderr);
    fixture.assert_not_pushed();
}

/// Single remote fallback.
#[test]
fn single_remote_fallback() {
    let fixture = Fixture::new();
    fixture.git(&["branch", "--unset-upstream"]);
    fixture.git(&["remote", "rename", "origin", "publish"]);
    fixture.git(&["config", "push.default", "current"]);
    let plan = fixture.invoke(&["--dry-run", "--base", "main"]);
    assert!(
        plan.stdout.contains("destination: publish/feature"),
        "{}",
        plan.stdout
    );
}

/// Tag collision preserves current branch identity.
#[test]
fn tag_collision_preserves_current_branch_identity() {
    let fixture = Fixture::new();
    fixture.git(&["tag", "feature", "main"]);
    fixture.git(&["branch", "heads/feature", "refs/heads/feature"]);
    fixture.git(&[
        "push",
        "origin",
        "refs/heads/heads/feature:refs/heads/heads/feature",
    ]);
    fixture.invoke(&["--base", "main"]);
    assert_eq!(
        fixture.git(&["symbolic-ref", "HEAD"]).stdout,
        "refs/heads/feature\n"
    );
    assert_eq!(fixture.rev("refs/heads/feature^"), fixture.base);
    assert_eq!(
        fixture.rev_at("refs/heads/heads/feature", &fixture.remote),
        fixture.old_feature
    );
    assert_eq!(
        fixture.rev("refs/heads/feature"),
        fixture.rev_at("refs/heads/feature", &fixture.remote)
    );
}

/// Unicode whitespace preserves current branch identity.
#[test]
fn unicode_whitespace_preserves_current_branch_identity() {
    let fixture = Fixture::new();
    let name = "feature\u{a0}";
    fixture.git(&["switch", "-c", name]);
    fixture.git(&[
        "push",
        "-u",
        "origin",
        &format!("refs/heads/{name}:refs/heads/{name}"),
    ]);
    fixture.invoke(&["--base", "main"]);
    assert_eq!(
        fixture.git(&["symbolic-ref", "HEAD"]).stdout,
        format!("refs/heads/{name}\n")
    );
    assert_eq!(fixture.rev(&format!("refs/heads/{name}^")), fixture.base);
    assert_eq!(
        fixture.rev(&format!("refs/heads/{name}")),
        fixture.rev_at(&format!("refs/heads/{name}"), &fixture.remote)
    );
    fixture.assert_not_pushed();
}

/// Unsupported remote configs fail before mutation.
#[test]
fn unsupported_remote_configs_fail_before_mutation() {
    let fixture = Fixture::new();
    let before = fixture.git(&["show-ref"]).stdout;
    let configurations = [
        ("remote.origin.mirror", "true"),
        ("remote.origin.pushurl", "different"),
        ("remote.origin.push", ":feature"),
        ("remote.origin.push", "refs/heads/*:refs/heads/*"),
    ];
    for (key, value) in configurations {
        fixture.git(&["config", key, value]);
        let result = fixture.invoke_result(&["--replace", "--base", "main", "feature:feature"]);
        assert_ne!(result.code, SUCCESS);
        assert_eq!(fixture.git(&["show-ref"]).stdout, before);
        fixture.assert_not_pushed();
        fixture.git(&["config", "--unset-all", key]);
    }
}

/// Upstream destination can have another name.
#[test]
fn upstream_destination_can_have_another_name() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "-c", "local", "feature"]);
    fixture.git(&["branch", "--set-upstream-to", "origin/feature"]);
    fixture.git(&["config", "push.default", "upstream"]);
    fixture.invoke(&["--replace", "--base", "main"]);
    assert_eq!(fixture.rev("local^"), fixture.base);
    assert_eq!(
        fixture.rev("local"),
        fixture.rev_at("feature", &fixture.remote)
    );
    assert_eq!(fixture.rev("feature"), fixture.old_feature);
}

/// Author identity and timezone survive normalization, base advances, and repeated pushes.
#[test]
fn metadata_is_stable_across_repeated_pushes_and_new_bases() {
    let fixture = Fixture::new();
    fixture.git(&[
        "commit",
        "--amend",
        "--no-edit",
        "--author",
        "Original Author <original@example.invalid>",
        "--date",
        "1000000000 +0530",
    ]);
    fixture.commit("second", "second", "Add second");
    fixture.git(&[
        "commit",
        "--amend",
        "--no-edit",
        "--author",
        "Second Author <second@example.invalid>",
        "--date",
        "1000000001 -0330",
    ]);
    fixture.git(&["branch", "sibling"]);
    fixture.commit("third", "third", "Add third");
    fixture.git(&[
        "commit",
        "--amend",
        "--no-edit",
        "--author",
        "Third Author <third@example.invalid>",
        "--date",
        "1000000010 +1245",
    ]);
    fixture.git(&["push", "--force", "origin", "feature"]);
    let merged = fixture.git(&["merge-tree", "--write-tree", "feature", "main"]);
    let expected_tree = merged.stdout.strip_suffix('\n').unwrap();
    fixture.invoke(&["--base", "main"]);
    let expected = "Original Author <original@example.invalid>|1000000000|2001-09-09 07:16:40 +0530|Original Author <original@example.invalid>|1000000000|2001-09-09 07:16:40 +0530\nSecond Author <second@example.invalid>|1000000001|2001-09-08 22:16:41 -0330|Second Author <second@example.invalid>|1000000001|2001-09-08 22:16:41 -0330\nThird Author <third@example.invalid>|1000000010|2001-09-09 14:31:50 +1245|Third Author <third@example.invalid>|1000000010|2001-09-09 14:31:50 +1245\n";
    let format = "--format=%an <%ae>|%at|%ai|%cn <%ce>|%ct|%ci";
    assert_eq!(
        fixture
            .git(&["log", "--reverse", format, "origin/main..feature"])
            .stdout,
        expected
    );
    assert_eq!(fixture.rev("sibling"), fixture.rev("feature^"));
    assert_eq!(fixture.rev("feature^{tree}"), expected_tree);
    let first = fixture.git(&["rev-list", "origin/main..feature"]).stdout;
    fixture.git(&["config", "user.name", "Different Operator"]);
    fixture.git(&["config", "user.email", "different@example.invalid"]);
    fixture.invoke(&["--base", "main"]);
    assert_eq!(
        fixture.git(&["rev-list", "origin/main..feature"]).stdout,
        first
    );
    assert_eq!(
        fixture.rev("feature"),
        fixture.rev_at("feature", &fixture.remote)
    );
    fixture.git(&["switch", "main"]);
    fixture.commit("new-base", "new base", "Advance base again");
    fixture.git(&["push", "origin", "main"]);
    fixture.git(&["switch", "feature"]);
    fixture.invoke(&["--base", "main"]);
    assert_eq!(
        fixture
            .git(&["log", "--reverse", format, "origin/main..feature"])
            .stdout,
        expected
    );
    assert_eq!(fixture.rev("sibling"), fixture.rev("feature^"));
    let rebased = fixture.git(&["rev-list", "origin/main..feature"]).stdout;
    assert_ne!(rebased, first);
    fixture.invoke(&["--base", "main"]);
    assert_eq!(
        fixture.git(&["rev-list", "origin/main..feature"]).stdout,
        rebased
    );
}

/// Replacement leaves siblings untouched while normalizing both metadata timestamps.
#[test]
fn replacement_metadata_is_idempotent() {
    let fixture = Fixture::new();
    fixture.git(&[
        "commit",
        "--amend",
        "--no-edit",
        "--date",
        "1000000000 +0000",
    ]);
    fixture.git(&["branch", "sibling"]);
    let sibling = fixture.rev("sibling");
    fixture.invoke(&["--replace", "--base", "main"]);
    let first = fixture.rev("feature");
    assert_eq!(
        fixture
            .git(&["show", "--no-patch", "--format=%at %ai|%ct %ci", "feature"])
            .stdout,
        "1000000000 2001-09-09 01:46:40 +0000|1000000000 2001-09-09 01:46:40 +0000\n"
    );
    fixture.invoke(&["--replace", "--base", "main"]);
    assert_eq!(fixture.rev("feature"), first);
    assert_eq!(fixture.rev("sibling"), sibling);
    assert_eq!(fixture.rev_at("feature", &fixture.remote), first);
}

/// A patch subsumed by the base must never cause the callback to rewrite upstream.
#[test]
fn dropped_first_commit_does_not_normalize_base() {
    let fixture = Fixture::new();
    fixture.git(&["switch", "main"]);
    fs::write(fixture.repo.join("feature.txt"), "feature").unwrap();
    fixture.git(&["add", "feature.txt"]);
    fixture.commit(
        "also-upstream",
        "upstream",
        "Include feature with another change",
    );
    fixture.git(&[
        "commit",
        "--amend",
        "--no-edit",
        "--date",
        "1000000000 +0530",
    ]);
    fixture.git(&["push", "origin", "main"]);
    let base = fixture.rev("main");
    fixture.git(&["switch", "feature"]);
    fixture.invoke(&["--replace", "--base", "main"]);
    assert_eq!(fixture.rev("feature"), base);
    assert_eq!(fixture.rev_at("feature", &fixture.remote), base);
    assert_eq!(fixture.rev_at("main", &fixture.remote), base);
    fixture.invoke(&["--replace", "--base", "main"]);
    assert_eq!(fixture.rev("feature"), base);
}

/// A metadata rewrite preserves message bytes, including deliberate whitespace.
#[test]
fn metadata_preserves_commit_message_bytes() {
    let fixture = Fixture::new();
    let message = fixture.root.path().join("message");
    fs::write(&message, "Subject  \n\nbody  \n\n\n").unwrap();
    fixture.git(&[
        "commit",
        "--amend",
        "--cleanup=verbatim",
        "--file",
        path(&message),
        "--date",
        "1000000000 +0530",
    ]);
    let before = fixture
        .git(&["show", "--no-patch", "--format=%B", "feature"])
        .stdout;
    fixture.invoke(&["--replace", "--base", "main"]);
    assert_eq!(
        fixture
            .git(&["show", "--no-patch", "--format=%B", "feature"])
            .stdout,
        before
    );
}

/// Continuing a conflicted rebase still normalizes metadata without publishing.
#[test]
fn conflict_continue_normalizes_metadata() {
    let fixture = Fixture::new();
    fixture.git(&[
        "commit",
        "--amend",
        "--no-edit",
        "--date",
        "1000000000 +0530",
    ]);
    fixture.git(&["switch", "main"]);
    fixture.commit("feature.txt", "conflicting base", "Conflict with feature");
    fixture.git(&["push", "origin", "main"]);
    fixture.git(&["switch", "feature"]);
    assert_eq!(
        fixture.invoke_result(&["--replace", "--base", "main"]).code,
        FAILURE
    );
    fs::write(fixture.repo.join("feature.txt"), "resolved").unwrap();
    fixture.git(&["add", "feature.txt"]);
    fixture.git(&["rebase", "--continue"]);
    assert_eq!(
        fixture
            .git(&["show", "--no-patch", "--format=%at %ai|%ct %ci", "feature"])
            .stdout,
        "1000000000 2001-09-09 07:16:40 +0530|1000000000 2001-09-09 07:16:40 +0530\n"
    );
    fixture.assert_not_pushed();
    let resumed = fixture.rev("feature");
    fixture.invoke(&["--replace", "--base", "main"]);
    assert_eq!(fixture.rev("feature"), resumed);
    assert_eq!(fixture.rev_at("feature", &fixture.remote), resumed);
}

/// Rebase callbacks quote executable paths containing shell syntax literally.
#[test]
fn metadata_callback_quotes_executable_path() {
    let fixture = Fixture::new();
    let executable = fixture.root.path().join("push ' $ (literal)");
    fs::copy(binary(), &executable).unwrap();
    fixture.command(path(&executable), &["--base", "main"], &fixture.repo);
    assert_eq!(
        fixture.rev("feature"),
        fixture.rev_at("feature", &fixture.remote)
    );
}
