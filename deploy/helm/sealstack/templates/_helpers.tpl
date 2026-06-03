{{/*
Common template helpers for the sealstack chart.
*/}}

{{- define "sealstack.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "sealstack.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "sealstack.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "sealstack.labels" -}}
helm.sh/chart: {{ include "sealstack.chart" . }}
{{ include "sealstack.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: sealstack
{{- end -}}

{{- define "sealstack.selectorLabels" -}}
app.kubernetes.io/name: {{ include "sealstack.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "sealstack.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "sealstack.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "sealstack.secretName" -}}
{{- if .Values.secret.create -}}
{{- default (printf "%s-credentials" (include "sealstack.fullname" .)) .Values.secret.name -}}
{{- else -}}
{{- default (printf "%s-credentials" (include "sealstack.fullname" .)) .Values.secret.name -}}
{{- end -}}
{{- end -}}
