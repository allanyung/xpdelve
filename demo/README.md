# xpdelve demo

This directory contains a disposable Crossplane environment used to record the
xpdelve animation in the project README. It runs on
[vCluster in Docker (vind)](https://www.vcluster.com/docs/vcluster/quick-start/docker)
and does not use the active Kubernetes context or any cloud provider.

The vind configuration disables its optional registry proxy and LoadBalancer
integration. Neither feature is needed by this demo, and disabling them keeps
cluster creation working in environments where Docker Hub access is restricted.

## Resource tree

The demo creates this nested Crossplane v2 trace:

```text
DemoPlatform/xpdelve-demo
├── DemoNetwork/xpdelve-demo-network
│   ├── ConfigMap/xpdelve-demo-network-settings
│   └── NetworkPolicy/xpdelve-demo-default-deny
└── DemoApplication/xpdelve-demo-app
    ├── Deployment/xpdelve-demo-web
    ├── Service/xpdelve-demo-web
    └── ConfigMap/xpdelve-demo-app-settings
```

The network settings ConfigMap has a demonstration finalizer. A foreground
delete of the root XR therefore pauses with the complete cascade visible.
Removing `demo.xpdelve.io/hold-for-recording` from xpdelve allows Crossplane to
finish deleting the tree.

## Prerequisites

- Docker with at least 4 GB of memory available
- `vcluster` 0.34 or newer
- Helm 3
- `kubectl`
- Crossplane CLI
- [VHS](https://github.com/charmbracelet/vhs) for recording

On macOS the missing demo-specific tools can be installed with:

```console
brew install loft-sh/tap/vcluster vhs
```

## Commands

Create the isolated cluster and install the demo:

```console
task demo:setup
```

The setup writes its kubeconfig to `target/demo/kubeconfig`. Scripts always
pass that file explicitly, so they cannot accidentally apply resources to the
current kubeconfig context.

Record the GIF after setup:

```console
task demo:record
```

The recording recreates the root XR, builds the release binary, and writes
`docs/assets/xpdelve.gif`. To recreate only the deleted resource tree, run
`task demo:reset`. To remove the entire vind cluster, run
`task demo:destroy`.
