# Implementation checklist

## Phase 1: trace explorer foundation

- [x] Initialize the independent repository and project metadata.
- [x] Record the agreed architecture and configuration contract.
- [x] Implement CLI parsing and configuration validation.
- [x] Execute and cancel bounded Crossplane trace processes.
- [x] Parse and project generic trace JSON into immutable tree snapshots.
- [x] Keep terminal input responsive while refresh work is active.
- [x] Implement tree navigation, collapse/expand, find, filter, and help.
- [x] Test parsing, identity, status, process failure, and interaction behavior.

## Phase 2: resilient reconciliation

- [x] Reconcile selection and expansion by stable identity.
- [x] Add retry backoff, refresh coalescing, and visible diagnostics.

## Phase 3: Kubernetes inspection and actions

- [x] Initialize a kube-rs client and dynamic discovery cache.
- [x] Add captured describe output, conditions, lazy events, and live YAML.
- [x] Add pause, unpause, delete, and selective finalizer removal.
- [x] Add action concurrency limits and identity revalidation.
- [x] Add permanent `kubectl edit`.

## Phase 4: release readiness

- [ ] Extend redaction beyond Secrets and add warning-gated reveal.
- [x] Sanitize untrusted text before terminal rendering.
- [x] Add OSC 52 clipboard and basic diagnostics.
- [ ] Add opt-in structured logging.
- [ ] Add terminal snapshots and mocked Kubernetes API integration tests.
- [x] Gate releases on matching SemVer tags, tests, and binary smoke checks.
- [x] Complete initial CLI, configuration, safety, and migration documentation.
- [x] Configure checksummed Linux GNU and macOS amd64/arm64 archives.
