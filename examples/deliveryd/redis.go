package main

import (
	"context"
	"crypto/tls"
	"fmt"
	"strings"
	"time"

	"github.com/redis/go-redis/v9"
)

// RedisClient wraps the Redis client for delivery state queries.
type RedisClient struct {
	client *redis.Client
}

// NewRedisClientFromEndpoint creates a new Redis client from an endpoint URL.
// Supports formats:
//   - redis://host:port
//   - rediss://host:port (TLS)
//   - host:port
func NewRedisClientFromEndpoint(endpoint string) (*RedisClient, error) {
	// TLS is enabled by default (matches the old TypeScript behavior)
	useTLS := true
	addr := endpoint

	// Parse protocol prefix
	// - rediss:// explicitly enables TLS (default)
	// - redis+insecure:// explicitly disables TLS (for local development)
	// - redis:// uses TLS by default (AWS MemoryDB requires TLS)
	if strings.HasPrefix(endpoint, "rediss://") {
		addr = strings.TrimPrefix(endpoint, "rediss://")
	} else if strings.HasPrefix(endpoint, "redis+insecure://") {
		useTLS = false
		addr = strings.TrimPrefix(endpoint, "redis+insecure://")
	} else if strings.HasPrefix(endpoint, "redis://") {
		addr = strings.TrimPrefix(endpoint, "redis://")
	}

	// Default port if not specified
	if !strings.Contains(addr, ":") {
		addr = addr + ":6379"
	}

	opts := &redis.Options{
		Addr:         addr,
		DialTimeout:  2 * time.Second,
		ReadTimeout:  3 * time.Second,
		WriteTimeout: 3 * time.Second,
	}
	if useTLS {
		opts.TLSConfig = &tls.Config{
			MinVersion: tls.VersionTLS12,
		}
	}

	return &RedisClient{
		client: redis.NewClient(opts),
	}, nil
}

// Close closes the Redis connection.
func (r *RedisClient) Close() error {
	return r.client.Close()
}

// Ping checks the Redis connection.
func (r *RedisClient) Ping(ctx context.Context) error {
	return r.client.Ping(ctx).Err()
}

// OutputSHAEntry represents a target's output SHA from Redis.
type OutputSHAEntry struct {
	Label     string
	OutputSHA string
}

// GetOutputSHAsForCommit scans Redis for all output-sha keys for a given commit.
// Key format: output-sha:{ci_host}:{commit_sha}:{workspace}:{label}
func (r *RedisClient) GetOutputSHAsForCommit(ctx context.Context, ciHost, commitSHA, workspace string) ([]OutputSHAEntry, error) {
	pattern := fmt.Sprintf("output-sha:%s:%s:%s:*", ciHost, commitSHA, workspace)
	var entries []OutputSHAEntry

	iter := r.client.Scan(ctx, 0, pattern, 0).Iterator()
	for iter.Next(ctx) {
		key := iter.Val()
		outputSHA, err := r.client.Get(ctx, key).Result()
		if err != nil {
			if err == redis.Nil {
				continue
			}
			return nil, fmt.Errorf("failed to get value for key %s: %w", key, err)
		}

		label := extractLabelFromKey(key, ciHost, commitSHA, workspace)
		entries = append(entries, OutputSHAEntry{
			Label:     label,
			OutputSHA: outputSHA,
		})
	}
	if err := iter.Err(); err != nil {
		return nil, fmt.Errorf("scan error: %w", err)
	}

	return entries, nil
}

// GetDeliverySignature gets the delivery signature for an output SHA.
// Key format: delivery-signature:{ci_host}:{output_sha}:{workspace}
// Returns the build URL if delivered, empty string if not.
func (r *RedisClient) GetDeliverySignature(ctx context.Context, ciHost, outputSHA, workspace string) (string, error) {
	key := fmt.Sprintf("delivery-signature:%s:%s:%s", ciHost, outputSHA, workspace)
	val, err := r.client.Get(ctx, key).Result()
	if err != nil {
		if err == redis.Nil {
			return "", nil
		}
		return "", fmt.Errorf("failed to get delivery signature: %w", err)
	}
	return val, nil
}

// extractLabelFromKey extracts the label from an output-sha key.
func extractLabelFromKey(key, ciHost, commitSHA, workspace string) string {
	prefix := fmt.Sprintf("output-sha:%s:%s:%s:", ciHost, commitSHA, workspace)
	return strings.TrimPrefix(key, prefix)
}

// SetOutputSHA records an output SHA for a target.
// Key format: output-sha:{ci_host}:{commit_sha}:{workspace}:{label}
func (r *RedisClient) SetOutputSHA(ctx context.Context, ciHost, commitSHA, workspace, label, outputSHA string) error {
	key := fmt.Sprintf("output-sha:%s:%s:%s:%s", ciHost, commitSHA, workspace, label)
	return r.client.Set(ctx, key, outputSHA, 0).Err()
}

// SetDeliverySignature marks an output SHA as delivered.
// Key format: delivery-signature:{ci_host}:{output_sha}:{workspace}
func (r *RedisClient) SetDeliverySignature(ctx context.Context, ciHost, outputSHA, workspace, signature string) error {
	key := fmt.Sprintf("delivery-signature:%s:%s:%s", ciHost, outputSHA, workspace)
	return r.client.Set(ctx, key, signature, 0).Err()
}

// DeleteArtifactMetadata deletes artifact metadata for an output SHA.
// Key format: {ci_host}:{output_sha}:{workspace}
func (r *RedisClient) DeleteArtifactMetadata(ctx context.Context, ciHost, outputSHA, workspace string) error {
	key := fmt.Sprintf("%s:%s:%s", ciHost, outputSHA, workspace)
	return r.client.Del(ctx, key).Err()
}
