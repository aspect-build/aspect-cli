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
	"fmt"
	"io"
	"net"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
	"golang.org/x/sync/errgroup"
	buildv1 "google.golang.org/genproto/googleapis/devtools/build/v1"
	"google.golang.org/grpc"
	"google.golang.org/protobuf/types/known/anypb"

	buildeventstream "github.com/aspect-build/aspect-cli/bazel/buildeventstream"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	grpc_mock "github.com/aspect-build/aspect-cli/pkg/aspectgrpc/mock"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system/besproxy"
	besproxy_mock "github.com/aspect-build/aspect-cli/pkg/plugin/system/besproxy/mock"
	stdlib_mock "github.com/aspect-build/aspect-cli/pkg/stdlib/mock"
)

func TestSetup(t *testing.T) {
	t.Run("fails when netListen fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctx := context.Background()

		listenErr := fmt.Errorf("failed listen")
		besBackend := &besBackend{
			netListen: func(network, address string) (net.Listener, error) {
				return nil, listenErr
			},
			ctx: ctx,
		}
		err := besBackend.Setup()

		g.Expect(err).To(MatchError(fmt.Errorf("failed to setup BES backend: %w", listenErr)))
	})

	t.Run("succeeds when netListen succeeds", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctx := context.Background()

		besBackend := &besBackend{
			netListen: func(network, address string) (net.Listener, error) {
				return nil, nil // It's fine to return nil for net.Listener as it doesn't get called in Setup.
			},
			ctx: ctx,
		}
		err := besBackend.Setup()

		g.Expect(err).To(BeNil())
	})
}

func TestServeWait(t *testing.T) {
	t.Run("fails when grpcServer.Serve fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		grpcServer := grpc_mock.NewMockServer(ctrl)
		serveErr := fmt.Errorf("failed serve")
		grpcServer.
			EXPECT().
			Serve(gomock.Any()).
			Return(serveErr).
			Times(1)
		addr := stdlib_mock.NewMockNetAddr(ctrl)
		addr.
			EXPECT().
			String().
			Return("127.0.0.1:12345").
			Times(1)
		listener := stdlib_mock.NewMockNetListener(ctrl)
		listener.
			EXPECT().
			Addr().
			Return(addr).
			Times(1)
		grpcDialer := grpc_mock.NewMockDialer(ctrl)
		grpcDialer.
			EXPECT().
			DialContext(gomock.Any(), "127.0.0.1:12345", gomock.Any(), gomock.Any()).
			Return(nil, fmt.Errorf("dial error")).
			AnyTimes()

		besBackend := &besBackend{
			grpcServer: grpcServer,
			listener:   listener,
			grpcDialer: grpcDialer,
		}
		err := besBackend.ServeWait(context.Background())

		g.Expect(err).To(MatchError(fmt.Errorf("failed to serve and wait BES backend: %w", serveErr)))
	})

	t.Run("fails when grpcDialer.DialContext exceeds timeout", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		grpcServer := grpc_mock.NewMockServer(ctrl)
		grpcServer.
			EXPECT().
			Serve(gomock.Any()).
			Return(nil).
			AnyTimes()
		addr := stdlib_mock.NewMockNetAddr(ctrl)
		addr.
			EXPECT().
			String().
			Return("127.0.0.1:12345").
			Times(1)
		listener := stdlib_mock.NewMockNetListener(ctrl)
		listener.
			EXPECT().
			Addr().
			Return(addr).
			Times(1)
		grpcDialer := grpc_mock.NewMockDialer(ctrl)
		grpcDialer.
			EXPECT().
			DialContext(gomock.Any(), "127.0.0.1:12345", gomock.Any(), gomock.Any()).
			Return(nil, context.DeadlineExceeded).
			Times(1)

		besBackend := &besBackend{
			grpcServer: grpcServer,
			listener:   listener,
			grpcDialer: grpcDialer,
		}
		err := besBackend.ServeWait(context.Background())

		g.Expect(err).To(MatchError(fmt.Errorf("failed to serve and wait BES backend: %w", context.DeadlineExceeded)))
	})

	t.Run("succeeds when grpcDialer.DialContext connects", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		grpcServer := grpc_mock.NewMockServer(ctrl)
		grpcServer.
			EXPECT().
			Serve(gomock.Any()).
			Return(nil).
			AnyTimes()
		addr := stdlib_mock.NewMockNetAddr(ctrl)
		addr.
			EXPECT().
			String().
			Return("127.0.0.1:12345").
			Times(1)
		listener := stdlib_mock.NewMockNetListener(ctrl)
		listener.
			EXPECT().
			Addr().
			Return(addr).
			Times(1)
		clientConn := grpc_mock.NewMockClientConn(ctrl)
		clientConn.
			EXPECT().
			Close().
			Return(nil).
			Times(1)
		grpcDialer := grpc_mock.NewMockDialer(ctrl)
		grpcDialer.
			EXPECT().
			DialContext(gomock.Any(), "127.0.0.1:12345", gomock.Any(), gomock.Any()).
			Return(clientConn, nil).
			Times(1)

		besBackend := &besBackend{
			grpcServer: grpcServer,
			listener:   listener,
			grpcDialer: grpcDialer,
		}
		err := besBackend.ServeWait(context.Background())

		g.Expect(err).To(BeNil())
	})
}

func TestGracefulStop(t *testing.T) {
	t.Run("calls grpcServer.GracefulStop and closes the listener", func(t *testing.T) {
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		grpcServer := grpc_mock.NewMockServer(ctrl)
		grpcServer.
			EXPECT().
			GracefulStop().
			Times(1)
		listener := stdlib_mock.NewMockNetListener(ctrl)
		listener.
			EXPECT().
			Close().
			Return(nil).
			Times(1)

		besBackend := &besBackend{
			grpcServer: grpcServer,
			listener:   listener,
		}
		besBackend.GracefulStop()
	})
}

func TestPublishBuildToolEventStream(t *testing.T) {
	t.Run("fails when stream.Recv fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		eventStream := grpc_mock.NewMockPublishBuildEvent_PublishBuildToolEventStreamServer(ctrl)
		expectedErr := fmt.Errorf("failed to receive")
		eventStream.
			EXPECT().
			Recv().
			Return(nil, expectedErr).
			Times(1)

		eventStream.
			EXPECT().
			Context().
			Return(context.Background()).
			Times(1)
		besBackend := &besBackend{
			ready: make(chan bool, 1),
		}
		close(besBackend.ready)
		err := besBackend.PublishBuildToolEventStream(eventStream)

		g.Expect(err).To(MatchError(fmt.Errorf("error receiving on build event stream from bazel server: %v", expectedErr)))
	})

	t.Run("fails when stream.Send fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		eventStream := grpc_mock.NewMockPublishBuildEvent_PublishBuildToolEventStreamServer(ctrl)
		event := &buildv1.BuildEvent{}
		streamId := &buildv1.StreamId{BuildId: "1"}
		orderedBuildEvent := &buildv1.OrderedBuildEvent{
			StreamId:       streamId,
			SequenceNumber: 1,
			Event:          event,
		}
		req := &buildv1.PublishBuildToolEventStreamRequest{OrderedBuildEvent: orderedBuildEvent}
		recv := eventStream.
			EXPECT().
			Recv().
			Return(req, nil).
			Times(1)
		eventStream.
			EXPECT().
			Recv().
			Return(nil, io.EOF).
			Times(1).
			After(recv)
		res := &buildv1.PublishBuildToolEventStreamResponse{
			StreamId:       req.OrderedBuildEvent.StreamId,
			SequenceNumber: req.OrderedBuildEvent.SequenceNumber,
		}
		expectedErr := fmt.Errorf("failed to send")
		eventStream.
			EXPECT().
			Send(res).
			Return(expectedErr).
			Times(1)

		eventStream.
			EXPECT().
			Context().
			Return(context.Background()).
			Times(1)
		besBackend := &besBackend{
			subscribers:   &subscriberList{},
			mtSubscribers: &subscriberList{},
			ready:         make(chan bool),
		}
		close(besBackend.ready)
		err := besBackend.PublishBuildToolEventStream(eventStream)

		g.Expect(err).To(MatchError(fmt.Errorf("error sending ack %v to bazel server: %v", 1, expectedErr)))
	})

	t.Run("succeeds when stream.Send fails but ignoreBesUploadErrors is true", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		eventStream := grpc_mock.NewMockPublishBuildEvent_PublishBuildToolEventStreamServer(ctrl)
		event := &buildv1.BuildEvent{}
		streamId := &buildv1.StreamId{BuildId: "1"}
		orderedBuildEvent := &buildv1.OrderedBuildEvent{
			StreamId:       streamId,
			SequenceNumber: 1,
			Event:          event,
		}
		req := &buildv1.PublishBuildToolEventStreamRequest{OrderedBuildEvent: orderedBuildEvent}
		recv := eventStream.
			EXPECT().
			Recv().
			Return(req, nil).
			Times(1)
		eventStream.
			EXPECT().
			Recv().
			Return(nil, io.EOF).
			Times(1).
			After(recv)
		res := &buildv1.PublishBuildToolEventStreamResponse{
			StreamId:       req.OrderedBuildEvent.StreamId,
			SequenceNumber: req.OrderedBuildEvent.SequenceNumber,
		}
		expectedErr := fmt.Errorf("failed to send")
		eventStream.
			EXPECT().
			Send(res).
			Return(expectedErr).
			Times(1)

		eventStream.
			EXPECT().
			Context().
			Return(context.Background()).
			Times(1)
		besBackend := &besBackend{
			subscribers:           &subscriberList{},
			mtSubscribers:         &subscriberList{},
			ignoreBesUploadErrors: true,
			ready:                 make(chan bool),
		}
		close(besBackend.ready)
		err := besBackend.PublishBuildToolEventStream(eventStream)

		g.Expect(err).To(Not(HaveOccurred()))
	})

	t.Run("succeeds without subscribers", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		eventStream := grpc_mock.NewMockPublishBuildEvent_PublishBuildToolEventStreamServer(ctrl)
		event := &buildv1.BuildEvent{}
		streamId := &buildv1.StreamId{BuildId: "1"}
		orderedBuildEvent := &buildv1.OrderedBuildEvent{
			StreamId:       streamId,
			SequenceNumber: 1,
			Event:          event,
		}
		req := &buildv1.PublishBuildToolEventStreamRequest{OrderedBuildEvent: orderedBuildEvent}
		recv := eventStream.
			EXPECT().
			Recv().
			Return(req, nil).
			Times(1)
		eventStream.
			EXPECT().
			Recv().
			Return(nil, io.EOF).
			Times(1).
			After(recv)
		res := &buildv1.PublishBuildToolEventStreamResponse{
			StreamId:       req.OrderedBuildEvent.StreamId,
			SequenceNumber: req.OrderedBuildEvent.SequenceNumber,
		}
		eventStream.
			EXPECT().
			Send(res).
			Return(nil).
			Times(1)

		eventStream.
			EXPECT().
			Context().
			Return(context.Background()).
			Times(1)
		besBackend := &besBackend{
			subscribers:   &subscriberList{},
			mtSubscribers: &subscriberList{},
			ready:         make(chan bool),
		}
		close(besBackend.ready)
		err := besBackend.PublishBuildToolEventStream(eventStream)

		g.Expect(err).To(Not(HaveOccurred()))
	})

	t.Run("succeeds with subscribers", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		eventStream := grpc_mock.NewMockPublishBuildEvent_PublishBuildToolEventStreamServer(ctrl)
		buildEvent := &buildeventstream.BuildEvent{}
		var anyBuildEvent anypb.Any
		anyBuildEvent.MarshalFrom(buildEvent)
		event := &buildv1.BuildEvent{Event: &buildv1.BuildEvent_BazelEvent{BazelEvent: &anyBuildEvent}}
		streamId := &buildv1.StreamId{BuildId: "1"}
		orderedBuildEvent := &buildv1.OrderedBuildEvent{
			StreamId:       streamId,
			SequenceNumber: 1,
			Event:          event,
		}
		req := &buildv1.PublishBuildToolEventStreamRequest{OrderedBuildEvent: orderedBuildEvent}
		recv := eventStream.
			EXPECT().
			Recv().
			Return(req, nil).
			Times(1)
		eventStream.
			EXPECT().
			Recv().
			Return(nil, io.EOF).
			Times(1).
			After(recv)
		res := &buildv1.PublishBuildToolEventStreamResponse{
			StreamId:       req.OrderedBuildEvent.StreamId,
			SequenceNumber: req.OrderedBuildEvent.SequenceNumber,
		}
		eventStream.
			EXPECT().
			Send(res).
			Return(nil).
			Times(1)

		besBackend := &besBackend{
			subscribers:   &subscriberList{},
			mtSubscribers: &subscriberList{},
			errors:        &aspecterrors.ErrorList{},
			ready:         make(chan bool),
		}
		close(besBackend.ready)
		var calledSubscriber1, calledSubscriber2, calledSubscriber3 bool
		besBackend.RegisterSubscriber(func(evt *buildeventstream.BuildEvent, sn int64) error {
			// g.Expect(evt).To(Equal(buildEvent))
			g.Expect(sn).To(Equal(orderedBuildEvent.SequenceNumber))
			calledSubscriber1 = true
			return nil
		}, false)
		expectedSubscriber2Err := fmt.Errorf("error from subscriber 2")
		besBackend.RegisterSubscriber(func(evt *buildeventstream.BuildEvent, sn int64) error {
			// g.Expect(evt).To(Equal(buildEvent))
			g.Expect(sn).To(Equal(orderedBuildEvent.SequenceNumber))
			calledSubscriber2 = true
			return expectedSubscriber2Err
		}, false)
		expectedSubscriber3Err := fmt.Errorf("error from subscriber 3")
		besBackend.RegisterSubscriber(func(evt *buildeventstream.BuildEvent, sn int64) error {
			// g.Expect(evt).To(Equal(buildEvent))
			g.Expect(sn).To(Equal(orderedBuildEvent.SequenceNumber))
			calledSubscriber3 = true
			return expectedSubscriber3Err
		}, false)

		eventStream.
			EXPECT().
			Context().
			Return(context.Background()).
			Times(1)
		err := besBackend.PublishBuildToolEventStream(eventStream)

		g.Expect(err).To(Not(HaveOccurred()))
		g.Expect(calledSubscriber1).To(BeTrue())
		g.Expect(calledSubscriber2).To(BeTrue())
		g.Expect(calledSubscriber3).To(BeTrue())

		subscriberErrs := besBackend.Errors()
		g.Expect(subscriberErrs[0]).To(MatchError(expectedSubscriber2Err))
		g.Expect(subscriberErrs[1]).To(MatchError(expectedSubscriber3Err))
	})

	t.Run("succeeds with proxies", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		ctx := context.Background()
		eventStream := grpc_mock.NewMockPublishBuildEvent_PublishBuildToolEventStreamServer(ctrl)
		buildEvent := &buildeventstream.BuildEvent{}
		var anyBuildEvent anypb.Any
		anyBuildEvent.MarshalFrom(buildEvent)
		event := &buildv1.BuildEvent{Event: &buildv1.BuildEvent_BazelEvent{BazelEvent: &anyBuildEvent}}
		streamId := &buildv1.StreamId{BuildId: "1"}
		orderedBuildEvent := &buildv1.OrderedBuildEvent{
			StreamId:       streamId,
			SequenceNumber: 1,
			Event:          event,
		}
		req := &buildv1.PublishBuildToolEventStreamRequest{OrderedBuildEvent: orderedBuildEvent}
		recv := eventStream.
			EXPECT().
			Recv().
			Return(req, nil).
			Times(1)
		eventStream.
			EXPECT().
			Recv().
			Return(nil, io.EOF).
			Times(1).
			After(recv)
		res := &buildv1.PublishBuildToolEventStreamResponse{
			StreamId:       req.OrderedBuildEvent.StreamId,
			SequenceNumber: req.OrderedBuildEvent.SequenceNumber,
		}
		eventStream.
			EXPECT().
			Send(res).
			Return(nil).
			Times(1)

		besBackend := &besBackend{
			besProxies:    []besproxy.BESProxy{},
			subscribers:   &subscriberList{},
			mtSubscribers: &subscriberList{},
			errors:        &aspecterrors.ErrorList{},
			ready:         make(chan bool),
		}
		close(besBackend.ready)

		_, egCtx := errgroup.WithContext(ctx)
		besProxy := besproxy_mock.NewMockBESProxy(ctrl)

		besProxy.
			EXPECT().
			PublishBuildToolEventStream(egCtx, grpc.WaitForReady(false)).
			Return(nil).
			Times(1)

		besBackend.RegisterBesProxy(egCtx, besProxy)

		besProxy.
			EXPECT().
			Healthy().
			Return(true).
			Times(5)
		besProxy.
			EXPECT().
			Send(req).
			Return(nil).
			Times(1)
		besProxy.
			EXPECT().
			CloseSend().
			Return(nil).
			Times(1)
		besProxy.
			EXPECT().
			Recv().
			Return(nil, nil).
			Times(1)
		besProxy.
			EXPECT().
			Recv().
			Return(nil, io.EOF).
			Times(1)
		eventStream.
			EXPECT().
			Context().
			Return(ctx).
			Times(1)
		err := besBackend.PublishBuildToolEventStream(eventStream)

		g.Expect(err).To(Not(HaveOccurred()))
	})
}
