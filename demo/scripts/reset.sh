#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
KUBECONFIG_PATH="$ROOT/target/demo/kubeconfig"
NAMESPACE=xpdelve-demo

if [ ! -f "$KUBECONFIG_PATH" ]; then
  printf 'demo kubeconfig not found; run task demo:setup first\n' >&2
  exit 1
fi

if kubectl --kubeconfig "$KUBECONFIG_PATH" get \
  demoplatform/xpdelve-demo \
  --namespace "$NAMESPACE" >/dev/null 2>&1; then
  kubectl --kubeconfig "$KUBECONFIG_PATH" delete \
    demoplatform/xpdelve-demo \
    --namespace "$NAMESPACE" \
    --wait=false

  kubectl --kubeconfig "$KUBECONFIG_PATH" patch \
    configmap/xpdelve-demo-network-settings \
    --namespace "$NAMESPACE" \
    --type=merge \
    --patch '{"metadata":{"finalizers":[]}}' >/dev/null 2>&1 || true

  kubectl --kubeconfig "$KUBECONFIG_PATH" wait \
    --for=delete \
    demoplatform/xpdelve-demo \
    --namespace "$NAMESPACE" \
    --timeout=2m
fi

kubectl --kubeconfig "$KUBECONFIG_PATH" apply -f "$ROOT/demo/manifests/04-instance.yaml"
kubectl --kubeconfig "$KUBECONFIG_PATH" wait \
  --for=condition=Ready \
  demoplatform/xpdelve-demo \
  --namespace "$NAMESPACE" \
  --timeout=5m

printf 'Demo resource tree reset\n'
