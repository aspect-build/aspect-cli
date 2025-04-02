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

	buildeventstream "github.com/aspect-build/aspect-cli/bazel/buildeventstream"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"github.com/aspect-build/aspect-cli/pkg/aspectgrpc"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system/besproxy"
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
	RegisterSubscriber(callback CallbackFn, multiThreaded bool)
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
	besProxies            []besproxy.BESProxy
	ctx                   context.Context
	errors                *aspecterrors.ErrorList
	errorsMutex           sync.RWMutex
	grpcDialer            aspectgrpc.Dialer
	grpcServer            aspectgrpc.Server
	listener              net.Listener
	netListen             func(network, address string) (net.Listener, error)
	startServe            chan struct{}
	subscribers           *subscriberList
	mtSubscribers         *subscriberList
	ignoreBesUploadErrors bool
}

// NewBESBackend creates a new Build Event Protocol backend.
func NewBESBackend(ctx context.Context) BESBackend {
	ignoreBesUploadErrors := os.Getenv("IGNORE_BES_UPLOAD_ERRORS") == "1"
	return &besBackend{
		besProxies:            []besproxy.BESProxy{},
		ctx:                   ctx,
		errors:                &aspecterrors.ErrorList{},
		grpcDialer:            aspectgrpc.NewDialer(),
		netListen:             net.Listen,
		startServe:            make(chan struct{}, 1),
		subscribers:           &subscriberList{},
		mtSubscribers:         &subscriberList{},
		ignoreBesUploadErrors: ignoreBesUploadErrors,
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
	bb.errorsMutex.RLock()
	defer bb.errorsMutex.RUnlock()
	return bb.errors.Errors()
}

// RegisterBesProxy registers a new build event stream proxy to send
// Build Event Protocol events to.
func (bb *besBackend) RegisterBesProxy(p besproxy.BESProxy) {
	bb.besProxies = append(bb.besProxies, p)
}

// CallbackFn is the signature for the callback function used by the subscribers
// of the Build Event Protocol events.
type CallbackFn func(*buildeventstream.BuildEvent, int64) error

// RegisterSubscriber registers a new subscriber callback function to the
// Build Event Protocol events.
func (bb *besBackend) RegisterSubscriber(callback CallbackFn, multiThreaded bool) {
	if multiThreaded {
		bb.mtSubscribers.Insert(callback)
	} else {
		bb.subscribers.Insert(callback)
	}
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
			if bb.ignoreBesUploadErrors {
				// Silence errors when reporting back to bazel
				err = nil
			}
			return err
		})
	}
	return &emptypb.Empty{}, eg.Wait()
}

func (bb *besBackend) SendEventsToSubscribers(c chan *buildv1.PublishBuildToolEventStreamRequest, subscribers *subscriberList) {
	for req := range c {
		// Forward the build event to subscribers
		if subscribers.head == nil {
			continue
		}
		event := req.GetOrderedBuildEvent().GetEvent()
		if event != nil {
			bazelEvent := event.GetBazelEvent()
			if bazelEvent != nil {
				var buildEvent *buildeventstream.BuildEvent = &buildeventstream.BuildEvent{}
				err := bazelEvent.UnmarshalTo(buildEvent)
				if err != nil {
					fmt.Fprintf(os.Stderr, "Error unmarshaling build event %v: %s\n", req.GetOrderedBuildEvent().GetSequenceNumber(), err.Error())
					continue
				}
				s := subscribers.head
				for s != nil {
					if err := s.callback(buildEvent, req.GetOrderedBuildEvent().GetSequenceNumber()); err != nil {
						bb.errorsMutex.Lock()
						bb.errors.Insert(err)
						bb.errorsMutex.Unlock()
					}
					s = s.next
				}
			}
		}
	}
}

// PublishBuildToolEventStream implements the gRPC PublishBuildToolEventStream
// service.
func (bb *besBackend) PublishBuildToolEventStream(
	stream buildv1.PublishBuildEvent_PublishBuildToolEventStreamServer,
) error {
	ctx := stream.Context()

	const numMultiSends = 10

	subChan := make(chan *buildv1.PublishBuildToolEventStreamRequest, 1000)
	subMultiChan := make(chan *buildv1.PublishBuildToolEventStreamRequest, 1000)
	fwdChan := make(chan *buildv1.PublishBuildToolEventStreamRequest, 1000)
	ackChan := make(chan *buildv1.PublishBuildToolEventStreamRequest, 1000)

	eg, egCtx := errgroup.WithContext(ctx)

	// Setup forwarding proxy streams
	for _, bp := range bb.besProxies {
		err := bp.PublishBuildToolEventStream(egCtx, grpc.WaitForReady(false))
		if err != nil {
			// If we fail to create the build event stream to a proxy then print out an error but don't fail the GRPC call
			fmt.Fprintf(os.Stderr, "Error creating build event stream to %v: %s\n", bp.Host(), err.Error())
		}
	}

	// Goroutine to receive messages from the Bazel server and send them to processing channels
	eg.Go(func() error {
		for {
			req, err := stream.Recv()
			if err != nil {
				close(subChan)
				close(subMultiChan)
				close(fwdChan)
				close(ackChan)
				if err == io.EOF {
					return nil
				}
				// If we fail to receive a BES event from bazel server then fail the GRPC call early
				// and surface this error back to Bazel; this is over localhost so should generally not
				// happen unless something has gone terribly wrong.
				return fmt.Errorf("error receiving on build event stream from bazel server: %v", err.Error())
			}
			subChan <- req
			subMultiChan <- req
			fwdChan <- req
			ackChan <- req
		}
	})

	// Goroutine to send acknowledgments back to the Bazel server
	eg.Go(func() error {
		for req := range ackChan {
			res := &buildv1.PublishBuildToolEventStreamResponse{
				StreamId:       req.OrderedBuildEvent.StreamId,
				SequenceNumber: req.OrderedBuildEvent.SequenceNumber,
			}
			err := stream.Send(res)
			if err != nil {
				// If we fail to send an ack back to the bazel server then fail the GRPC call early
				// since Bazel will hang waiting for all acks; this is over localhost so should generally not
				// happen unless something has gone terribly wrong.
				return fmt.Errorf("error sending ack %v to bazel server: %v", res.SequenceNumber, err.Error())
			}
		}
		return nil
	})

	// Goroutines to process messages and send to subscribers
	eg.Go(func() error { bb.SendEventsToSubscribers(subChan, bb.subscribers); return nil })
	for i := 0; i < numMultiSends; i++ {
		eg.Go(func() error { bb.SendEventsToSubscribers(subMultiChan, bb.mtSubscribers); return nil })
	}

	// Goroutines to receive acks from BES proxies
	for _, bp := range bb.besProxies {
		if !bp.StreamCreated() {
			continue
		}
		proxy := bp // make a copy of the BESProxy before passing into the go-routine below.
		eg.Go(func() error {
			for {
				_, err := proxy.Recv()
				if err != nil {
					if err != io.EOF {
						// If we fail to recv an ack from a proxy then print out an error but don't fail the GRPC call
						fmt.Fprintf(os.Stderr, "Error receiving build event stream ack %v: %s\n", proxy.Host(), err.Error())
					}
					break
				}
			}
			return nil
		})
	}

	// Goroutine to forward to build event to BES proxies
	eg.Go(func() error {
		for fwd := range fwdChan {
			for _, bp := range bb.besProxies {
				if !bp.StreamCreated() {
					continue
				}
				err := bp.Send(fwd)
				if err != nil {
					// If we fail to send to a proxy then print out an error but don't fail the GRPC call
					fmt.Fprintf(os.Stderr, "Error sending build event to %v: %s\n", bp.Host(), err.Error())
				}
			}
		}
		for _, bp := range bb.besProxies {
			if !bp.StreamCreated() {
				continue
			}
			if err := bp.CloseSend(); err != nil {
				fmt.Fprintf(os.Stderr, "Error closing build event stream to %v: %s\n", bp.Host(), err.Error())
			}
		}
		return nil
	})

	err := eg.Wait()

	if bb.ignoreBesUploadErrors {
		// Silence errors when reporting back to bazel
		err = nil
	}

	return err
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
