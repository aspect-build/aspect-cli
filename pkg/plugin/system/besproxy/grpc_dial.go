/*
 * Copyright 2023 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package besproxy

import (
	"context"
	"crypto/x509"
	"fmt"
	"math"
	"net/url"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/keepalive"
)

type grpcHeaders struct {
	headers map[string]string
}

func (c *grpcHeaders) GetRequestMetadata(ctx context.Context, uri ...string) (map[string]string, error) {
	return c.headers, nil
}

func (c *grpcHeaders) RequireTransportSecurity() bool {
	return false
}

func grpcDial(host string, headers map[string]string) (*grpc.ClientConn, error) {
	opts := []grpc.DialOption{
		grpc.WithPerRPCCredentials(&grpcHeaders{headers: headers}),
		grpc.WithDefaultCallOptions(
			// Bazel doesn't seem to set a maximum send message size, therefore
			// we match the default send message for Go, which should be enough
			// for all messages sent by Bazel (roughly 2.14GB).
			grpc.MaxCallRecvMsgSize(math.MaxInt32),
			// Here we are just being explicit with the default value since we
			// also set the receive message size.
			grpc.MaxCallSendMsgSize(math.MaxInt32),
		),
		grpc.WithKeepaliveParams(keepalive.ClientParameters{
			Time:                40 * time.Second,
			Timeout:             15 * time.Second,
			PermitWithoutStream: true,
		}),
	}
	var transportCreds credentials.TransportCredentials
	if p, err := url.Parse(host); err == nil {
		if p.Scheme == "grpcs" {
			// TODO(f0rmiga): allow for custom cert to be injected via config. Big enterprises usually have
			// their own CA certs and they often will want to consume separately from the system certs, aka
			// well-known CA certs (the most widely used list comes from Mozilla, and curl compiles it here:
			// https://curl.se/ca/cacert.pem).
			pool, err := x509.SystemCertPool()
			if err != nil {
				return nil, fmt.Errorf("failed to initialize GOOGLE gRPC dial options: %w", err)
			}
			// TODO(f0rmiga): allow serverNameOverride from configuration file.
			transportCreds = credentials.NewClientTLSFromCert(pool, "")
			host = p.Host
			if p.Port() == "" {
				host += ":443"
			}
		} else if p.Scheme != "unix" {
			host = p.Host
		}
	}
	if transportCreds == nil {
		transportCreds = insecure.NewCredentials()
	}
	opts = append(opts, grpc.WithTransportCredentials(transportCreds))
	return grpc.Dial(host, opts...)
}
