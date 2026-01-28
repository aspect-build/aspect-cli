package main

// QueryRequest represents a request to query delivery state for a commit.
type QueryRequest struct {
	CIHost    string `json:"ci_host"`
	CommitSHA string `json:"commit_sha"`
	Workspace string `json:"workspace"`
}

// TargetStatus represents the delivery status of a single target.
type TargetStatus struct {
	Label       string  `json:"label"`
	OutputSHA   string  `json:"output_sha"`
	Delivered   bool    `json:"delivered"`
	DeliveredBy *string `json:"delivered_by,omitempty"`
}

// QueryResponse contains the delivery state for all targets in a commit.
type QueryResponse struct {
	CIHost    string         `json:"ci_host"`
	CommitSHA string         `json:"commit_sha"`
	Workspace string         `json:"workspace"`
	Targets   []TargetStatus `json:"targets"`
}

// RecordRequest represents a request to record an output SHA for a target.
type RecordRequest struct {
	CIHost    string `json:"ci_host"`
	CommitSHA string `json:"commit_sha"`
	Workspace string `json:"workspace"`
	Label     string `json:"label"`
	OutputSHA string `json:"output_sha"`
}

// DeliverRequest represents a request to mark a target as delivered.
type DeliverRequest struct {
	CIHost    string `json:"ci_host"`
	OutputSHA string `json:"output_sha"`
	Workspace string `json:"workspace"`
	Signature string `json:"signature"` // e.g., build URL that performed the delivery
}

// DeleteArtifactRequest represents a request to delete artifact metadata.
type DeleteArtifactRequest struct {
	CIHost    string `json:"ci_host"`
	OutputSHA string `json:"output_sha"`
	Workspace string `json:"workspace"`
}
