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
	"io"

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
	StreamCreated() bool
	Healthy() bool
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
	hadError int32

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
	ev, err := bp.client.PublishLifecycleEvent(ctx, req)
	if err != nil {
		return ev, bp.trackError(fmt.Errorf("failed calling PublishLifecycleEvent to %v: %w", bp.host, err))
	}
	return ev, nil
}

func (bp *besProxy) PublishBuildToolEventStream(ctx context.Context, opts ...grpc.CallOption) error {
	if bp.client == nil {
		return fmt.Errorf("not connected to %v", bp.host)
	}
	s, err := bp.client.PublishBuildToolEventStream(ctx, opts...)
	if err != nil {
		bp.trackError(err)
		return fmt.Errorf("failed calling PublishBuildToolEventStream to %v: %w", bp.host, err)
	}
	bp.stream = s
	return nil
}

func (bp *besProxy) StreamCreated() bool {
	return bp.stream != nil
}

func (bp *besProxy) Send(req *buildv1.PublishBuildToolEventStreamRequest) error {
	if bp.stream == nil {
		return fmt.Errorf("stream to %v not configured", bp.host)
	}

	err := bp.stream.Send(req)

	// EOF indicates the server sent an error which must be received.
	if err == io.EOF {
		_, err = bp.Recv()
	}

	return bp.trackError(err)
}

func (bp *besProxy) Recv() (*buildv1.PublishBuildToolEventStreamResponse, error) {
	if bp.stream == nil {
		return nil, fmt.Errorf("stream to %v not configured", bp.host)
	}
	resp, err := bp.stream.Recv()
	return resp, bp.trackError(err)
}

func (bp *besProxy) CloseSend() error {
	if bp.stream == nil {
		return nil
	}
	err := bp.stream.CloseSend()
	bp.stream = nil
	return err
}

// TrackError tracks errors and marks the stream as unhealthy if too many errors occur.
func (bp *besProxy) trackError(err error) error {
	if err != nil {
		bp.hadError++
		if bp.hadError == 5 {
			fmt.Printf("stream to %s is marked unhealthy, taking out of rotation.", bp.host)
		}
	}
	return err
}

func (bp *besProxy) Healthy() bool {
	return bp.hadError < 5 && bp.stream != nil
}
