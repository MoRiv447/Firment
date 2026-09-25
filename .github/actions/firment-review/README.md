# Firment review — a GitHub Action

Reviews a pull request with Firment and posts one comment: the built-in rules over the files
the PR changed, plus the dependency graph.

```yaml
name: review
on: pull_request

permissions:
  contents: read
  pull-requests: write

jobs:
  firment:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0        # the action needs the base commit to diff against
      - uses: MoRiv447/Firment/.github/actions/firment-review@main
        with:
          deps: 'true'          # `firm review deps --markdown` (licences + advisories)
          comment: 'true'
          fail-on-high: 'false' # report first, gate later
```

## What it runs, and why those two

| Step | Command | What it is for |
|---|---|---|
| Changed files | `git diff --name-only <base>...HEAD -- '*.rs' '*.c' '*.h' '*.s'` | makes the report about *this* change rather than about the repository |
| Static review | `firm review <paths> --markdown` | the built-in rules: an `unsafe` block with no stated reason, a blocking call in an interrupt handler, `clippy` diagnostics |
| Dependencies | `firm review deps --markdown` | the licence list, and advisories when `cargo-audit` is installed |

Both reviews **exit 2 when something high is found**. That is a signal, not a crash, so the
action captures the code, puts it in the report, and decides per `fail-on-high`. A review
that silently passed because the shell treated exit 2 as a failure would be the worst of
both: no gate and no report.

The comment is **updated in place** (`<!-- firment-review -->` marks it) rather than appended:
a review that adds a comment per push trains people to stop reading them.

## Inputs

| Input | Default | Notes |
|---|---|---|
| `version` | `latest` | a Firment release tag; the action installs it with the repository's own `install.sh` |
| `paths` | *(the PR's changed files)* | pass `.` to review everything, or a directory |
| `deps` | `true` | skipped automatically when there is no `Cargo.toml` |
| `comment` | `true` | needs `pull-requests: write` |
| `fail-on-high` | `true` | makes the job fail (the report is still posted first) |

## Limits, stated rather than discovered

- **`ubuntu-latest` only.** The install step uses `install.sh`; a Windows runner would need
  `install.ps1`, which is not wired yet.
- **No network, no advisories.** `firm review deps` reports advisories only when `cargo-audit`
  is installed *and* can reach the advisory database. In a sandbox without network it reports
  licences and says so — which is why the output is in the report rather than assumed.
- **The static rules are not a model review.** They are deterministic and free; the
  model-backed review is `firm review last`, which needs a provider and a key, and is not
  part of this action.

## Using it from a repository that is not Firment

The action installs Firment from a release, so no checkout of this repository is needed:
reference it by path in this repository (`MoRiv447/Firment/.github/actions/firment-review@main`)
or copy the directory into the consuming repository.

## Publishing it as `firment-action` (not done)

The plan (§5, item 6) asks for a published action. It is **not published**, and the honest
reason is that publishing needs a repository and a release that this environment cannot
create. What it takes, exactly:

1. Create `MoRiv447/firment-action` with `action.yml` (this file), `README.md`, and `LICENSE`.
2. Tag it (`v1`) so consumers can pin `@v1` rather than `@main`.
3. In the Marketplace listing form, pick the categories; the action needs no build step
   because it installs a released binary.
4. Keep the `firm review` invocations in sync with the CLI — the drift guard is a test in
   `crates/firment-cli/src/main.rs`
   (`the_github_action_is_structurally_sound_and_calls_commands_that_exist`), which reads
   **`action.yml`** (not this file) and fails if it names a subcommand the binary does not
   have.
