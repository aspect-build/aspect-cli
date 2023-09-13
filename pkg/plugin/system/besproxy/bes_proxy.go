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
