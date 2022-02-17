/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package aspectgrpc

import (
	"context"
	"net"

	"google.golang.org/grpc"
)

// Server is an interface for the upstream grpc.Server struct.
type Server interface {
	Serve(lis net.Listener) error
	GracefulStop()
}

// Dialer is an interface for the upstream grpc.DialContext function.
type Dialer interface {
	DialContext(ctx context.Context, target string, opts ...grpc.DialOption) (conn ClientConn, err error)
}

// dialer wraps the upstream grpc.DialContext function, satisfying the Dialer
// interface.
type dialer struct{}

func (*dialer) DialContext(ctx context.Context, target string, opts ...grpc.DialOption) (conn ClientConn, err error) {
	return grpc.DialContext(ctx, target, opts...)
}

// NewDialer creates a new Dialer with the dialer wrapper.
func NewDialer() Dialer {
	return &dialer{}
}

// ClientConn is an interface for the upstream grpc.ClientConn struct.
type ClientConn interface {
	Close() error
}
