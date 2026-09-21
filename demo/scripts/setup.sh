#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
CLUSTER=xpdelve-demo
KUBECONFIG_PATH="$ROOT/target/demo/kubeconfig"

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

need docker
need vcluster
need helm
need kubectl
need crossplane

if ! docker info >/dev/null 2>&1; then
  printf 'Docker is not running\n' >&2
  exit 1
fi

mkdir -p "$(dirname "$KUBECONFIG_PATH")"

if ! vcluster list --driver docker --output json 2>/dev/null | grep -Eq "\"Name\"[[:space:]]*:[[:space:]]*\"$CLUSTER\""; then
  vcluster create "$CLUSTER" \
    --driver docker \
    --connect=false \
    --values "$ROOT/demo/vcluster.yaml"
fi

tmp="$KUBECONFIG_PATH.tmp"
trap 'rm -f "$tmp"' EXIT
vcluster connect "$CLUSTER" --driver docker --print >"$tmp"
mv "$tmp" "$KUBECONFIG_PATH"
trap - EXIT
chmod 600 "$KUBECONFIG_PATH"

helm upgrade --install crossplane crossplane \
  --repo https://charts.crossplane.io/stable \
  --version 2.3.3 \
  --namespace crossplane-system \
  --create-namespace \
  --kubeconfig "$KUBECONFIG_PATH" \
  --wait \
  --timeout 5m

kubectl --kubeconfig "$KUBECONFIG_PATH" apply -f "$ROOT/demo/manifests/00-rbac.yaml"
kubectl --kubeconfig "$KUBECONFIG_PATH" apply -f "$ROOT/demo/manifests/01-function.yaml"
kubectl --kubeconfig "$KUBECONFIG_PATH" wait \
  --for=condition=Healthy \
  function/function-patch-and-transform \
  --timeout=5m

kubectl --kubeconfig "$KUBECONFIG_PATH" apply -f "$ROOT/demo/manifests/02-xrds.yaml"
kubectl --kubeconfig "$KUBECONFIG_PATH" wait \
  --for=condition=Established \
  --all compositeresourcedefinitions.apiextensions.crossplane.io \
  --timeout=2m
kubectl --kubeconfig "$KUBECONFIG_PATH" apply -f "$ROOT/demo/manifests/03-compositions.yaml"
kubectl --kubeconfig "$KUBECONFIG_PATH" apply -f "$ROOT/demo/manifests/04-instance.yaml"
kubectl --kubeconfig "$KUBECONFIG_PATH" wait \
  --for=condition=Ready \
  demoplatform/xpdelve-demo \
  --namespace xpdelve-demo \
  --timeout=5m

KUBECONFIG="$KUBECONFIG_PATH" crossplane resource trace \
  demoplatform.demo.xpdelve.io/xpdelve-demo \
  --namespace xpdelve-demo \
  --output json >/dev/null

printf '\nDemo ready. Run: task demo:record\n'
