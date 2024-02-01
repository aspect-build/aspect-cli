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
	"fmt"

	"google.golang.org/grpc"
	"google.golang.org/protobuf/types/known/emptypb"

	buildv1 "google.golang.org/genproto/googleapis/devtools/build/v1"
)

const proxyChannelBufferSize = 10000

// BESProxy implements a Build Event Protocol backend to be passed to the
// `bazel build` command so that the Aspect plugins can register as subscribers
// to the build events.
type BESProxy interface {
	CloseSend() error
	Connect() error
	Host() string
	PublishBuildToolEventStream(ctx context.Context, opts ...grpc.CallOption) error
	PublishLifecycleEvent(ctx context.Context, req *buildv1.PublishLifecycleEventRequest, opts ...grpc.CallOption) (*emptypb.Empty, error)
	Recv() (*buildv1.PublishBuildToolEventStreamResponse, error)
	Send(req *buildv1.PublishBuildToolEventStreamRequest) error
}

func NewBesProxy(host string, headers map[string]string) *besProxy {
	return &besProxy{
		host:    host,
		headers: headers,
	}
}

type besProxy struct {
	client  buildv1.PublishBuildEventClient
	stream  buildv1.PublishBuildEvent_PublishBuildToolEventStreamClient
	host    string
	headers map[string]string
}

func (bp *besProxy) Connect() error {
	c, err := grpcDial(bp.host, bp.headers)
	if err != nil {
		return fmt.Errorf("failed to connect to build event stream backend %s: %w", bp.host, err)
	}
	bp.client = buildv1.NewPublishBuildEventClient(c)
	return nil
}

func (bp *besProxy) Host() string {
	return bp.host
}

func (bp *besProxy) PublishLifecycleEvent(ctx context.Context, req *buildv1.PublishLifecycleEventRequest, opts ...grpc.CallOption) (*emptypb.Empty, error) {
	if bp.client == nil {
		return &emptypb.Empty{}, fmt.Errorf("not connected to %v", bp.host)
	}
	return bp.client.PublishLifecycleEvent(ctx, req)
}

func (bp *besProxy) PublishBuildToolEventStream(ctx context.Context, opts ...grpc.CallOption) error {
	if bp.client == nil {
		return fmt.Errorf("not connected to %v", bp.host)
	}
	s, err := bp.client.PublishBuildToolEventStream(ctx, opts...)
	if err != nil {
		return fmt.Errorf("failed to create build event stream to %v: %w", bp.host, err)
	}
	bp.stream = s
	return nil
}

func (bp *besProxy) Send(req *buildv1.PublishBuildToolEventStreamRequest) error {
	if bp.stream == nil {
		return nil
	}

	// If we want to mutate the BES events in the future before they are sent out to external consumers, this is the place
	// to do it. See https://github.com/aspect-build/silo/blob/7f13ab16fa10ffcec71b09737f0370f22a508823/cli/core/pkg/plugin/system/besproxy/bes_proxy.go#L103
	// as an example.

	return bp.stream.Send(req)
}

func (bp *besProxy) Recv() (*buildv1.PublishBuildToolEventStreamResponse, error) {
	if bp.stream == nil {
		return nil, fmt.Errorf("stream to %v not configured", bp.host)
	}
	return bp.stream.Recv()
}

func (bp *besProxy) CloseSend() error {
	if bp.stream == nil {
		return nil
	}
	return bp.stream.CloseSend()
}
