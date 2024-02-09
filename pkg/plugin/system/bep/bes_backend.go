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
	"os"
	"sync"

	"github.com/golang/protobuf/ptypes/empty"
	"golang.org/x/sync/errgroup"
	buildv1 "google.golang.org/genproto/googleapis/devtools/build/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/protobuf/types/known/emptypb"

	buildeventstream "aspect.build/cli/bazel/buildeventstream"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/aspectgrpc"
	"aspect.build/cli/pkg/plugin/system/besproxy"
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
type BESBackend interface {
	Setup(opts ...grpc.ServerOption) error
	ServeWait(ctx context.Context) error
	GracefulStop()
	Addr() string
	RegisterBesProxy(p besproxy.BESProxy)
	RegisterSubscriber(callback CallbackFn)
	Errors() []error
}

// BESBackendFromContext extracts a BESBackend from the given context. It panics
// if the context doesn't have a BESBackend set up.
func BESBackendFromContext(ctx context.Context) BESBackend {
	return ctx.Value(besBackendInterceptorKey).(BESBackend)
}

func HasBESBackend(ctx context.Context) bool {
	return ctx.Value(besBackendInterceptorKey) != nil
}

func BESErrors(ctx context.Context) []error {
	if !HasBESBackend(ctx) {
		return []error{}
	}
	return BESBackendFromContext(ctx).Errors()
}

// InjectBESBackend injects the given BESBackend into the context.
func InjectBESBackend(ctx context.Context, besBackend BESBackend) context.Context {
	return context.WithValue(ctx, besBackendInterceptorKey, besBackend)
}

type besBackend struct {
	besProxies  []besproxy.BESProxy
	closeOnce   sync.Once
	ctx         context.Context
	errors      *aspecterrors.ErrorList
	grpcDialer  aspectgrpc.Dialer
	grpcServer  aspectgrpc.Server
	listener    net.Listener
	netListen   func(network, address string) (net.Listener, error)
	startServe  chan struct{}
	subscribers *subscriberList
}

// NewBESBackend creates a new Build Event Protocol backend.
func NewBESBackend(ctx context.Context) BESBackend {
	return &besBackend{
		besProxies:  []besproxy.BESProxy{},
		ctx:         ctx,
		errors:      &aspecterrors.ErrorList{},
		grpcDialer:  aspectgrpc.NewDialer(),
		netListen:   net.Listen,
		startServe:  make(chan struct{}, 1),
		subscribers: &subscriberList{},
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
			conn, err := bb.grpcDialer.DialContext(
				ctx,
				serverAddr,
				grpc.WithTransportCredentials(insecure.NewCredentials()),
				grpc.WithBlock(),
			)
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

// RegisterBesProxy registers a new build even stream proxy to send
// Build Event Protocol events to.
func (bb *besBackend) RegisterBesProxy(p besproxy.BESProxy) {
	bb.besProxies = append(bb.besProxies, p)
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
func (bb *besBackend) PublishLifecycleEvent(
	ctx context.Context,
	req *buildv1.PublishLifecycleEventRequest,
) (*empty.Empty, error) {
	// Forward to proxy clients
	eg, ctx := errgroup.WithContext(ctx)
	for _, c := range bb.besProxies {
		client := c
		eg.Go(func() error {
			_, err := client.PublishLifecycleEvent(ctx, req)
			return err
		})
	}
	return &emptypb.Empty{}, eg.Wait()
}

// PublishBuildToolEventStream implements the gRPC PublishBuildToolEventStream
// service.
func (bb *besBackend) PublishBuildToolEventStream(
	stream buildv1.PublishBuildEvent_PublishBuildToolEventStreamServer,
) error {
	ctx := stream.Context()

	// Setup forwarding proxy streams
	eg, ctx := errgroup.WithContext(ctx)
	for _, bp := range bb.besProxies {
		// Make a copy of the BESProxy before passing into the go-routine below.
		proxy := bp
		err := bp.PublishBuildToolEventStream(ctx, grpc.WaitForReady(false))
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error creating build event stream to %v: %s\n", proxy.Host(), err.Error())
			continue
		}
		eg.Go(func() error {
			for {
				_, err := proxy.Recv()
				if err == io.EOF {
					break
				}
				if err != nil {
					return fmt.Errorf("error receiving build event stream ack %v: %s\n", proxy.Host(), err.Error())
				}
			}
			return nil
		})
	}
	defer bb.closeBesProxies()

	for {
		// Wait for a build event
		req, err := stream.Recv()
		if err == io.EOF {
			// Close BES proxy streams and wait for acks
			bb.closeBesProxies()
			return eg.Wait()
		}
		if err != nil {
			return err
		}

		// Forward the build event to grpc outStreams
		for _, bp := range bb.besProxies {
			err := bp.Send(req)
			if err != nil {
				fmt.Fprintf(os.Stderr, "Error sending build event to %v: %s\n", bp.Host(), err.Error())
			}
		}

		// Forward the build event to subscribers
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

		// Ack the message
		res := &buildv1.PublishBuildToolEventStreamResponse{
			StreamId:       req.OrderedBuildEvent.StreamId,
			SequenceNumber: req.OrderedBuildEvent.SequenceNumber,
		}
		if err := stream.Send(res); err != nil {
			return err
		}
	}
}

func (bb *besBackend) closeBesProxies() {
	bb.closeOnce.Do(func() {
		for _, bp := range bb.besProxies {
			bp.CloseSend()
		}
	})
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
