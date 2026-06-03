# SealStack Helm chart

Deploy the SealStack gateway on Kubernetes.

## Quickstart

```bash
helm install sealstack ./deploy/helm/sealstack \
  --namespace sealstack --create-namespace \
  --set externalDatabase.url="postgres://user:pass@db:5432/sealstack" \
  --set externalQdrant.url="http://qdrant:6334"
```

Then either port-forward or wire up Ingress:

```bash
kubectl port-forward -n sealstack svc/sealstack 7070:7070
curl http://localhost:7070/v1/schemas
```

## Required externalised state

The chart **does not** ship Postgres or Qdrant — point at existing clusters:

- `externalDatabase.url` (or `externalDatabase.urlSecretRef.name/key`) — Postgres DSN
- `externalQdrant.url` (or `externalQdrant.urlSecretRef.name/key`) — Qdrant gRPC URL

Leave Qdrant unset to run the in-memory vector store (good for evaluation; data is lost on restart).

## Secrets

`secret.create=false` (the default) means the chart **references** but does not own a Secret named `<release>-credentials`. Provision it externally (recommended: `external-secrets` or `sealed-secrets`) with whatever your embedder / reranker needs:

```bash
kubectl create secret generic sealstack-credentials -n sealstack \
  --from-literal=OPENAI_API_KEY=sk-... \
  --from-literal=VOYAGE_API_KEY=...
```

Setting `secret.create=true` lets Helm own the Secret — **NOT recommended for production** because the values land in your release history.

## Policy bundles

`SEALSTACK_POLICY_BACKEND=wasm` (default) or `cedar`. Mount the bundle directory at `SEALSTACK_POLICY_DIR` via an extra ConfigMap or PVC; the chart doesn't ship one because policy authoring varies by team.

## Observability

`serviceMonitor.enabled=true` renders a Prometheus ServiceMonitor pointing at `/metrics`. Requires the kube-prometheus operator CRDs.

## Autoscaling + PDB

`autoscaling.enabled=true` renders an HPA against CPU and memory. `podDisruptionBudget.enabled=true` renders a PDB so the gateway survives node drains.

## Security defaults

- Non-root uid 10001
- Read-only root filesystem (with an emptyDir at `/tmp`)
- `capabilities.drop: [ALL]`, `seccompProfile: RuntimeDefault`
- `allowPrivilegeEscalation: false`

Adjust under `podSecurityContext` / `securityContext` if your cluster's PSA policy demands otherwise.

## Configuration reference

See `values.yaml` for the full list with inline comments.
