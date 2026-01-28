package main

import (
	"encoding/json"
	"net/http"
)

// Handler holds the HTTP handlers and their dependencies.
type Handler struct {
	redis *RedisClient
}

// NewHandler creates a new Handler with the given Redis client.
func NewHandler(redis *RedisClient) *Handler {
	return &Handler{redis: redis}
}

// HandleQuery handles POST /query requests.
func (h *Handler) HandleQuery(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	var req QueryRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "Invalid JSON: "+err.Error(), http.StatusBadRequest)
		return
	}

	if req.CIHost == "" || req.CommitSHA == "" || req.Workspace == "" {
		http.Error(w, "Missing required fields: ci_host, commit_sha, workspace", http.StatusBadRequest)
		return
	}

	ctx := r.Context()

	entries, err := h.redis.GetOutputSHAsForCommit(ctx, req.CIHost, req.CommitSHA, req.Workspace)
	if err != nil {
		http.Error(w, "Redis error: "+err.Error(), http.StatusInternalServerError)
		return
	}

	var targets []TargetStatus
	for _, entry := range entries {
		signature, err := h.redis.GetDeliverySignature(ctx, req.CIHost, entry.OutputSHA, req.Workspace)
		if err != nil {
			http.Error(w, "Redis error: "+err.Error(), http.StatusInternalServerError)
			return
		}

		target := TargetStatus{
			Label:     entry.Label,
			OutputSHA: entry.OutputSHA,
			Delivered: signature != "",
		}
		if signature != "" {
			target.DeliveredBy = &signature
		}
		targets = append(targets, target)
	}

	resp := QueryResponse{
		CIHost:    req.CIHost,
		CommitSHA: req.CommitSHA,
		Workspace: req.Workspace,
		Targets:   targets,
	}

	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(resp); err != nil {
		http.Error(w, "Failed to encode response", http.StatusInternalServerError)
		return
	}
}

// HandleHealth handles GET /health requests.
func (h *Handler) HandleHealth(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	ctx := r.Context()
	if err := h.redis.Ping(ctx); err != nil {
		w.WriteHeader(http.StatusServiceUnavailable)
		w.Write([]byte("unhealthy: redis connection failed"))
		return
	}

	w.WriteHeader(http.StatusOK)
	w.Write([]byte("ok"))
}

// HandleRecord handles POST /record requests to record an output SHA for a target.
func (h *Handler) HandleRecord(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	var req RecordRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "Invalid JSON: "+err.Error(), http.StatusBadRequest)
		return
	}

	if req.CIHost == "" || req.CommitSHA == "" || req.Workspace == "" || req.Label == "" || req.OutputSHA == "" {
		http.Error(w, "Missing required fields: ci_host, commit_sha, workspace, label, output_sha", http.StatusBadRequest)
		return
	}

	ctx := r.Context()
	if err := h.redis.SetOutputSHA(ctx, req.CIHost, req.CommitSHA, req.Workspace, req.Label, req.OutputSHA); err != nil {
		http.Error(w, "Redis error: "+err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusOK)
	w.Write([]byte("ok"))
}

// HandleDeliver handles POST /deliver requests to mark a target as delivered.
func (h *Handler) HandleDeliver(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	var req DeliverRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "Invalid JSON: "+err.Error(), http.StatusBadRequest)
		return
	}

	if req.CIHost == "" || req.OutputSHA == "" || req.Workspace == "" || req.Signature == "" {
		http.Error(w, "Missing required fields: ci_host, output_sha, workspace, signature", http.StatusBadRequest)
		return
	}

	ctx := r.Context()
	if err := h.redis.SetDeliverySignature(ctx, req.CIHost, req.OutputSHA, req.Workspace, req.Signature); err != nil {
		http.Error(w, "Redis error: "+err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusOK)
	w.Write([]byte("ok"))
}

// HandleDeleteArtifact handles POST /artifact/delete requests to delete artifact metadata.
func (h *Handler) HandleDeleteArtifact(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	var req DeleteArtifactRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "Invalid JSON: "+err.Error(), http.StatusBadRequest)
		return
	}

	if req.CIHost == "" || req.OutputSHA == "" || req.Workspace == "" {
		http.Error(w, "Missing required fields: ci_host, output_sha, workspace", http.StatusBadRequest)
		return
	}

	ctx := r.Context()
	if err := h.redis.DeleteArtifactMetadata(ctx, req.CIHost, req.OutputSHA, req.Workspace); err != nil {
		http.Error(w, "Redis error: "+err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusOK)
	w.Write([]byte("ok"))
}
