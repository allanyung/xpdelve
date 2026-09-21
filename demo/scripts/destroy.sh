#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
CLUSTER=xpdelve-demo

if ! command -v vcluster >/dev/null 2>&1; then
  printf 'missing required command: vcluster\n' >&2
  exit 1
fi

if vcluster list --driver docker --output json 2>/dev/null | grep -Eq "\"Name\"[[:space:]]*:[[:space:]]*\"$CLUSTER\""; then
  vcluster delete "$CLUSTER" --driver docker
fi

rm -f "$ROOT/target/demo/kubeconfig"
printf 'Demo cluster removed\n'
