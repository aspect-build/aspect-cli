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
	"strings"
	"sync"

	"github.com/fatih/color"
	"github.com/golang/protobuf/ptypes/empty"
	"golang.org/x/sync/errgroup"
	buildv1 "google.golang.org/genproto/googleapis/devtools/build/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
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
	RegisterBesProxy(ctx context.Context, p besproxy.BESProxy)
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

// Creates a channel where receiver receives nothing until the
// ready channel passed in as a parameter closes.
func bufferUntilReadyChan[T any](in <-chan T, ready <-chan bool) <-chan T {
	var buf []T
	out := make(chan T, 1000)

	go func() {
		defer close(out)
		for {
			select {
			case e, ok := <-in:
				// not ready, buffer events.
				if ok {
					buf = append(buf, e)
				}
			case <-ready:
				// got the ready signal, move to flush_buffer to start
				// releasing events received prior to ready event.
				goto flush_buffer
			}
		}

	flush_buffer:
		for _, v := range buf {
			// flush out the buffered events
			out <- v
		}
		// drop buffered events.
		buf = nil

		// forward events until channel closes.
		for v := range in {
			out <- v
		}
	}()
	return out
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
	ready                 chan bool
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
		ready:                 make(chan bool, 1),
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
func (bb *besBackend) RegisterBesProxy(ctx context.Context, p besproxy.BESProxy) {
	bb.besProxies = append(bb.besProxies, p)
	err := p.PublishBuildToolEventStream(ctx, grpc.WaitForReady(false))
	if err != nil {
		// If we fail to create the build event stream to a proxy then print out an error but don't fail the GRPC call
		fmt.Fprintf(os.Stderr, "Error creating build event stream to %v: %s\n", p.Host(), err.Error())
	}
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

	broadCastEvent := func(ctx context.Context, req *buildv1.PublishLifecycleEventRequest) error {
		eg, ctx := errgroup.WithContext(ctx)
		for _, c := range bb.besProxies {
			client := c
			if !client.Healthy() {
				continue
			}

			eg.Go(func() error {
				_, err := client.PublishLifecycleEvent(ctx, req)
				if bb.ignoreBesUploadErrors {
					// Silence errors when reporting back to bazel
					err = nil
				}
				return err
			})
		}
		return eg.Wait()
	}
	select {
	// https://aspect-build.slack.com/archives/C03LXS06TA7/p1746657294747679
	// If we know the upstream backends, forward right away.
	case <-bb.ready:
		// Forward to proxy clients
		return &emptypb.Empty{}, broadCastEvent(ctx, req)
	default:
		// If we don't know the upstream backends, so schedule a goroutine
		// and wait until it is ready and forward the events.
		go func() {
			<-bb.ready
			broadCastEvent(ctx, req)
		}()
		// We don't want bazel to wait before sending us more event.
		return &emptypb.Empty{}, nil
	}
}

func (bb *besBackend) SendEventsToSubscribers(c <-chan *buildv1.PublishBuildToolEventStreamRequest, subscribers *subscriberList) {
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

func (bb *besBackend) setupBesUpstreamBackends(ctx context.Context, optionsparsed *buildeventstream.OptionsParsed) error {
	backends := []string{}
	backendSeen := make(map[string]bool)
	globalRemoteHeaders := make(map[string]string)
	scopedRemoteHeaders := map[string]map[string]string{}

	hasNoWaitForUpload := false

	// Parse backends first to build up that map, then parse headers
	for _, arg := range optionsparsed.CmdLine {
		if strings.HasPrefix(arg, "--bes_backend=") {
			// Always skip our bes_backend to avoid recursive uploads.
			if arg == fmt.Sprintf("--bes_backend=%s", bb.Addr()) {
				continue
			}

			backend := strings.TrimLeft(arg, "--bes_backend=")
			if backendSeen[backend] {
				continue
			}
			backendSeen[backend] = true
			backends = append(backends, backend)
		} else if strings.HasPrefix(arg, "--remote_header=") || strings.HasPrefix(arg, "--bes_header=") {
			rawValue := strings.TrimPrefix(strings.TrimPrefix(arg, "--bes_header="), "--remote_header=")
			remoteHeader := strings.SplitN(rawValue, "=", 3)
			if len(remoteHeader) > 3 || len(remoteHeader) < 2 {
				return fmt.Errorf("invalid --remote_header flag value '%v'; value must be in the form of a 'name=value' assignment", arg)
			}

			// Decide which backend the header belongs to.
			backend := ""
			key := remoteHeader[0]
			value := remoteHeader[1]
			if len(remoteHeader) == 3 {
				backend = remoteHeader[2]
			}

			if backend == "" {
				globalRemoteHeaders[key] = value
			} else {
				// Append if the header already exists.
				if headers, ok := scopedRemoteHeaders[backend]; ok {
					// Append the new value to the existing value with a comma separator
					if prevValue, exists := headers[key]; exists {
						headers[key] = prevValue + ", " + value
					} else {
						headers[key] = value
					}
				} else {
					// Initialize the headers map if it doesn't exist.
					headers := map[string]string{
						key: value,
					}
					scopedRemoteHeaders[backend] = headers
				}
			}
		} else if strings.HasPrefix(arg, "--bes_upload_mode=") {
			mode := arg[len("--bes_upload_mode="):]
			hasNoWaitForUpload = mode == "nowait_for_upload_complete" || mode == "fully_async"
		}
	}

	if hasNoWaitForUpload {
		fmt.Fprintf(
			os.Stderr,
			"%s --bes_upload_mode nowait_for_upload_complete|fully_async may lead to incomplete BES uploads with Aspect CLI\n\t"+
				"See: https://github.com/aspect-build/aspect-cli/issues/851\n",
			color.YellowString("WARNING:"),
		)
	}

	if len(backends) > 0 {
		fmt.Fprintf(
			os.Stderr,
			"%s BES backends: %s. Forwarding to all.\n",
			color.GreenString("INFO:"),
			strings.Join(backends, ", "),
		)
	}

	for _, backend := range backends {
		headers := make(map[string]string)
		for key, value := range globalRemoteHeaders {
			headers[key] = value
		}
		if scoped, ok := scopedRemoteHeaders[backend]; ok {
			for key, value := range scoped {
				headers[key] = value
			}
		}
		besProxy := besproxy.NewBesProxy(backend, headers)
		if err := besProxy.Connect(); err != nil {
			fmt.Fprintf(os.Stderr, "Failed to connect to build event stream backend %s: %s", backend, err.Error())
		} else {
			bb.RegisterBesProxy(ctx, besProxy)
		}
	}
	close(bb.ready)
	return nil
}

// PublishBuildToolEventStream implements the gRPC PublishBuildToolEventStream
// service.
func (bb *besBackend) PublishBuildToolEventStream(
	stream buildv1.PublishBuildEvent_PublishBuildToolEventStreamServer,
) error {
	ctx := stream.Context()

	eg, egCtx := errgroup.WithContext(ctx)

	const numMultiSends = 10

	subChan := make(chan *buildv1.PublishBuildToolEventStreamRequest, 1000)
	subMultiChan := make(chan *buildv1.PublishBuildToolEventStreamRequest, 1000)
	fwdChan := make(chan *buildv1.PublishBuildToolEventStreamRequest, 1000)
	ackChan := make(chan *buildv1.PublishBuildToolEventStreamRequest, 1000)

	subChanRead := bufferUntilReadyChan(subChan, bb.ready)
	subMultiChanRead := bufferUntilReadyChan(subMultiChan, bb.ready)
	fwdChanRead := bufferUntilReadyChan(fwdChan, bb.ready)

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
			be := req.OrderedBuildEvent.Event.GetBazelEvent()
			if be != nil {
				var event *buildeventstream.BuildEvent = &buildeventstream.BuildEvent{}
				err := be.UnmarshalTo(event)
				if err != nil {
					fmt.Fprintf(os.Stderr, "Error unmarshaling build event %v: %s\n", req.GetOrderedBuildEvent().GetSequenceNumber(), err.Error())
					continue
				}
				if event.Id != nil {
					switch event.Id.Id.(type) {
					case *buildeventstream.BuildEventId_OptionsParsed:
						// Received options event, setup bes upstream backends based off commandline arguments bazel reported.
						// setup upstream backends async to prevent bazel client from waiting for upstream bes connections.
						eg.Go(func() error {
							return bb.setupBesUpstreamBackends(egCtx, event.GetOptionsParsed())
						})
					}
				}
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
	eg.Go(func() error { bb.SendEventsToSubscribers(subChanRead, bb.subscribers); return nil })
	for i := 0; i < numMultiSends; i++ {
		eg.Go(func() error { bb.SendEventsToSubscribers(subMultiChanRead, bb.mtSubscribers); return nil })
	}

	eg.Go(func() error {
		// Wait for ready event to start receiving acks from BES upstream proxies.
		<-bb.ready
		// Goroutines to receive acks from BES proxies
		for _, bp := range bb.besProxies {
			if !bp.Healthy() {
				continue
			}
			proxy := bp // make a copy of the BESProxy before passing into the go-routine below.
			eg.Go(func() error {
				for {
					// If the proxy is not healthy, break out of the loop
					if !proxy.Healthy() {
						break
					}
					_, err := proxy.Recv()
					if err != nil {
						if err != io.EOF {
							if status.Code(err) == codes.Canceled {
								break
							}
							// If we fail to recv an ack from a proxy then print out an error but don't fail the GRPC call
							fmt.Fprintf(os.Stderr, "error receiving build event stream ack %v: %s\n", proxy.Host(), err.Error())
						}
						break
					}
				}
				return nil
			})
		}
		return nil
	})

	// Goroutine to forward to build event to BES proxies
	eg.Go(func() error {
		for fwd := range fwdChanRead {
			for _, bp := range bb.besProxies {
				if !bp.Healthy() {
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
			if !bp.Healthy() {
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
