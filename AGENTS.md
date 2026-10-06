# Repository guidance

## Commands and verification

- Rust is pinned to **1.98.1** with Clippy/rustfmt in `rust-toolchain.toml`; native builds need a C compiler/linker and CMake (TLS uses aws-lc).
- `Taskfile.yml` is the contributor interface. `task build`, `task test`, and `task run -- Kind/name` use Cargo with `--locked`; `task fmt` formats sources.
- `task check` runs formatting check, Clippy, and tests as parallel dependencies. `task ci` runs **fmt-check → lint → test → release build** sequentially. Lint is `cargo clippy --locked --all-targets -- -D warnings`.
- Focus tests with `task test -- --lib app::tests::mutation_failure_opens_persistent_error_modal` (substitute the test/module filter); run CLI integration tests with `task test -- --test cli`.
- Most tests are inline module tests; UI tests are in `src/app/tests.rs` and use Ratatui `TestBackend`, not a real terminal. The normal suite needs no cluster or Crossplane CLI; trace process tests invoke `/bin/sh` and exercise Unix process-group cancellation.

## Execution boundaries

- This is one Cargo package with a library and binary. `src/main.rs` handles CLI/config setup; `src/app.rs` defines shared UI types and re-exports `src/app/runtime.rs::run`. Runtime owns the Tokio event loop; input/state/render behavior is split into its private sibling modules.
- `src/trace.rs` runs the external Crossplane trace command; `src/model.rs` parses/projects snapshots. Live YAML, events, and mutations use kube-rs in `src/kubernetes.rs`; Describe and Edit use kubectl through the app runtime.
- Keep trace parsing off the input/render path (`spawn_blocking`), rendering side-effect-free, and completion events generation-checked. Only one trace process runs at a time; refresh requests coalesce, and the previous snapshot stays usable during refresh/failure.
- `Identity` equality/hash deliberately exclude API **version** and UID: selection/expansion track group/kind/namespace/name. UID separately detects recreation and guards mutations.
- `--cmd` is shell-word parsing, **not shell execution**. Trace stdout must be one complete JSON document; stderr is captured separately. Do not introduce shell interpolation or merge the streams.

## Runtime and safety

- When adding or changing keybindings, update the `README.md` Keys section, `docs/keys.md`, and in-app help (`HELP_LINES` in `src/app.rs`) in the same change. Update relevant footer hints in `src/app/render.rs` and keybinding/UI tests in `src/app/tests.rs`; keep footer entries grouped by navigation/display controls, discovery, and resource actions.

- Live tracing requires **both stdin and stdout to be terminals**; there is no stdin/file trace mode or `trace` subcommand. For noninteractive smoke checks use `task run -- version` or `task run -- --help`.
- The app is writable by default. For inspection use `task run -- --readonly --context CONTEXT Kind/name`; runtime needs Crossplane CLI and kubeconfig access, plus kubectl for Describe/Edit.
- Preserve native mutation safeguards: re-fetch and match trace UID; deletes use UID/resourceVersion preconditions, patches atomically test both. Keep read-only gating, confirmation flows, and per-resource/global mutation limits.
- Preserve Secret YAML redaction and terminal-text sanitization (`src/kubernetes.rs`, `src/text.rs`); other resource kinds are not recursively credential-redacted. Details: `docs/safety.md`.
- Configuration uses `CLI > config > defaults`, validates unknown fields strictly, and defaults to `~/.config/xpdelve/config.toml`. Config changes/reload must retain CLI overrides; theme persistence must preserve unrelated settings. See `docs/configuration.md`.

## Operational gotchas

- `task install` downloads the latest published release; it does **not** install the local build. `task package` builds the local release binary.
- Live demo setup is separate from tests: `task demo:setup` creates a disposable vind cluster using Docker/vcluster/Helm. Keep explicit `target/demo/kubeconfig` arguments in demo scripts so they never target the active context. `task demo:record` resets the root resource and overwrites `docs/assets/xpdelve.gif`; prerequisites are in `demo/README.md`.
- `task release VERSION=x.y.z` requires clean `main`, updates manifest/lockfile, runs CI, and **creates a commit and annotated tag**; it does not push. Pushing `v*` tags triggers release publication, with tag/package version matching enforced.
- Adapted xpdig/Sofka code carries source attribution comments and entries in `NOTICE`; preserve these and record materially adapted code there. Architecture rationale is in `docs/design.md`.
