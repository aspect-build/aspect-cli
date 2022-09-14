/*
 * Copyright 2022 Aspect Build Systems, Inc.
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

package bep

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/url"

	"github.com/golang/protobuf/ptypes/empty"
	buildv1 "google.golang.org/genproto/googleapis/devtools/build/v1"
	"google.golang.org/grpc"

	buildeventstream "aspect.build/cli/bazel/buildeventstream"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/aspectgrpc"
)

// besBackendInterceptorKeyType is a type for the BESBackendInterceptorKey that
// avoids collisions.
type besBackendInterceptorKeyType byte

// besBackendInterceptorKey is the key for the injected BES backend into
// the context.
const besBackendInterceptorKey besBackendInterceptorKeyType = 0x00

// BESBackend implements a Build Event Protocol backend to be passed to the
// `bazel build` command so that the Aspect plugins can register as subscribers
// to the build events.
// TODO(f0rmiga): implement a forwarding client to an upstream BES backend if
// the user provides one to the `aspect build` command.
type BESBackend interface {
	Setup(opts ...grpc.ServerOption) error
	ServeWait(ctx context.Context) error
	GracefulStop()
	Addr() string
	RegisterSubscriber(callback CallbackFn)
	Errors() []error
}

// BESBackendFromContext extracts a BESBackend from the given context. It panics
// if the context doesn't have a BESBackend set up.
func BESBackendFromContext(ctx context.Context) BESBackend {
	return ctx.Value(besBackendInterceptorKey).(BESBackend)
}

// InjectBESBackend injects the given BESBackend into the context.
func InjectBESBackend(ctx context.Context, besBackend BESBackend) context.Context {
	return context.WithValue(ctx, besBackendInterceptorKey, besBackend)
}

type besBackend struct {
	subscribers *subscriberList
	errors      *aspecterrors.ErrorList
	listener    net.Listener
	grpcServer  aspectgrpc.Server
	startServe  chan struct{}
	netListen   func(network, address string) (net.Listener, error)
	grpcDialer  aspectgrpc.Dialer
}

// NewBESBackend creates a new Build Event Protocol backend.
func NewBESBackend() BESBackend {
	return &besBackend{
		subscribers: &subscriberList{},
		errors:      &aspecterrors.ErrorList{},
		startServe:  make(chan struct{}, 1),
		netListen:   net.Listen,
		grpcDialer:  aspectgrpc.NewDialer(),
	}
}

// Setup sets up the gRPC server.
func (bb *besBackend) Setup(opts ...grpc.ServerOption) error {
	// Never expose this to the network.
	lis, err := bb.netListen("tcp", "127.0.0.1:0")
	if err != nil {
		return fmt.Errorf("failed to setup BES backend: %w", err)
	}
	bb.listener = lis
	grpcServer := grpc.NewServer(opts...)
	bb.grpcServer = grpcServer
	buildv1.RegisterPublishBuildEventServer(grpcServer, bb)
	return nil
}

// ServeWait starts and waits for the gRPC services to be served.
func (bb *besBackend) ServeWait(ctx context.Context) error {
	errs := make(chan error, 1)
	go func() {
		if err := bb.grpcServer.Serve(bb.listener); err != nil {
			errs <- err
		}
	}()
	serverAddr := bb.listener.Addr().String()
	for {
		select {
		case err := <-errs:
			return fmt.Errorf("failed to serve and wait BES backend: %w", err)
		default:
			conn, err := bb.grpcDialer.DialContext(ctx, serverAddr, grpc.WithInsecure(), grpc.WithBlock())
			if err != nil {
				if errors.Is(err, context.DeadlineExceeded) {
					return fmt.Errorf("failed to serve and wait BES backend: %w", err)
				}
				continue
			}
			defer conn.Close()
		}
		return nil
	}
}

// GracefulStop stops the gRPC server gracefully by waiting for all the clients
// to disconnect.
func (bb *besBackend) GracefulStop() {
	defer bb.listener.Close()
	bb.grpcServer.GracefulStop()
}

// Addr returns the address for the gRPC server. Since the address is determined
// by the OS based on an available port at the time the gRPC server starts, this
// method returns the address to be used to construct the `bes_backend` flag
// passed to the `bazel (build|test|run)` commands. The address includes the
// scheme (protocol).
func (bb *besBackend) Addr() string {
	url := url.URL{
		Scheme: "grpc",
		Host:   bb.listener.Addr().String(),
	}
	return url.String()
}

// Errors return the errors produced by the subscriber callback functions.
func (bb *besBackend) Errors() []error {
	return bb.errors.Errors()
}

// CallbackFn is the signature for the callback function used by the subscribers
// of the Build Event Protocol events.
type CallbackFn func(*buildeventstream.BuildEvent) error

// RegisterSubscriber registers a new subscriber callback function to the
// Build Event Protocol events.
func (bb *besBackend) RegisterSubscriber(callback CallbackFn) {
	bb.subscribers.Insert(callback)
}

// PublishLifecycleEvent implements the gRPC PublishLifecycleEvent service.
func (*besBackend) PublishLifecycleEvent(
	ctx context.Context,
	req *buildv1.PublishLifecycleEventRequest,
) (*empty.Empty, error) {
	return &empty.Empty{}, nil
}

// PublishBuildToolEventStream implements the gRPC PublishBuildToolEventStream
// service.
func (bb *besBackend) PublishBuildToolEventStream(
	stream buildv1.PublishBuildEvent_PublishBuildToolEventStreamServer,
) error {
	for {
		req, err := stream.Recv()
		if err == io.EOF {
			return nil
		}
		if err != nil {
			return err
		}
		event := req.OrderedBuildEvent.Event
		if event != nil {
			bazelEvent := event.GetBazelEvent()
			if bazelEvent != nil {
				var buildEvent buildeventstream.BuildEvent
				if err := bazelEvent.UnmarshalTo(&buildEvent); err != nil {
					return err
				}

				s := bb.subscribers.head
				for s != nil {
					if err := s.callback(&buildEvent); err != nil {
						bb.errors.Insert(err)
					}
					s = s.next
				}
			}
		}
		res := &buildv1.PublishBuildToolEventStreamResponse{
			StreamId:       req.OrderedBuildEvent.StreamId,
			SequenceNumber: req.OrderedBuildEvent.SequenceNumber,
		}
		if err := stream.Send(res); err != nil {
			return err
		}
	}
}

// SubscriberList is a linked list for the Build Event Protocol event
// subscribers.
type subscriberList struct {
	head *subscriberNode
	tail *subscriberNode
}

// Insert inserts a new Build Event Protocol event callback into the linked
// list.
func (l *subscriberList) Insert(callback CallbackFn) {
	node := &subscriberNode{callback: callback}
	if l.head == nil {
		l.head = node
	} else {
		l.tail.next = node
	}
	l.tail = node
}

type subscriberNode struct {
	next     *subscriberNode
	callback CallbackFn
}
