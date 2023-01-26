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

package bep_fanner

import (
	"context"
	"io"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	buildv1_mock "github.com/aspect-build/silo/third_party/googleapis/google/devtools/build/v1/mock"
	buildv1 "google.golang.org/genproto/googleapis/devtools/build/v1"
)

var _ = Describe("BEP Collector", func() {
	var ctrl *gomock.Controller

	BeforeEach(func() {
		ctrl = gomock.NewController(GinkgoT())
	})

	AfterEach(func() {
		ctrl.Finish()
	})

	It("should configure the provided ConfigStreams", func() {
		bepFanner := NewBEPFanner()
		configStreams := []*ConfigStream{
			NewConfigStream("foo", nil),
			NewConfigStream("bar", nil),
		}
		err := bepFanner.Configure(configStreams)
		Expect(err).To(BeNil())
	})

	It("should forward PublishLifecycleEvent to the configured streams", func() {
		bepFanner := NewBEPFanner()
		configStreams := []*ConfigStream{
			NewConfigStream("foo", nil),
			NewConfigStream("bar", nil),
		}
		err := bepFanner.Configure(configStreams)
		Expect(err).To(BeNil())

		client1 := buildv1_mock.NewMockPublishBuildEventClient(ctrl)
		bepFanner.bepClients[0].PublishBuildEventClient = client1
		client2 := buildv1_mock.NewMockPublishBuildEventClient(ctrl)
		bepFanner.bepClients[1].PublishBuildEventClient = client2

		ctx := context.Background()
		req := &buildv1.PublishLifecycleEventRequest{}

		client1.EXPECT().PublishLifecycleEvent(ctx, req).Times(1)
		client2.EXPECT().PublishLifecycleEvent(ctx, req).Times(1)

		res, err := bepFanner.PublishLifecycleEvent(ctx, req)
		Expect(err).To(BeNil())
		Expect(res).ToNot(BeNil())
	})

	It("should forward three build events to the configured streams", func() {
		bepFanner := NewBEPFanner()
		configStreams := []*ConfigStream{
			NewConfigStream("foo", nil),
			NewConfigStream("bar", nil),
		}
		err := bepFanner.Configure(configStreams)
		Expect(err).To(BeNil())

		client1 := buildv1_mock.NewMockPublishBuildEventClient(ctrl)
		bepFanner.bepClients[0].PublishBuildEventClient = client1
		client2 := buildv1_mock.NewMockPublishBuildEventClient(ctrl)
		bepFanner.bepClients[1].PublishBuildEventClient = client2

		ctx := context.Background()

		req := &buildv1.PublishBuildToolEventStreamRequest{
			OrderedBuildEvent: &buildv1.OrderedBuildEvent{
				StreamId: &buildv1.StreamId{
					BuildId:      "who needs a BuildId?",
					InvocationId: "who needs an InvocationId?",
				},
				SequenceNumber: 23,
				Event:          &buildv1.BuildEvent{},
			},
		}
		res := &buildv1.PublishBuildToolEventStreamResponse{
			StreamId:       req.OrderedBuildEvent.StreamId,
			SequenceNumber: req.OrderedBuildEvent.SequenceNumber,
		}

		streamClient1 := buildv1_mock.NewMockPublishBuildEvent_PublishBuildToolEventStreamClient(ctrl)
		streamClient1.EXPECT().Recv().Times(3).Return(nil, nil)
		streamClient1.EXPECT().Recv().Times(1).Return(nil, io.EOF)
		streamClient1.EXPECT().Send(req).Times(3).Return(nil)
		streamClient1.EXPECT().CloseSend().Times(1).Return(nil)
		client1.EXPECT().PublishBuildToolEventStream(ctx).Times(1).Return(streamClient1, nil)

		streamClient2 := buildv1_mock.NewMockPublishBuildEvent_PublishBuildToolEventStreamClient(ctrl)
		streamClient2.EXPECT().Recv().Times(3).Return(nil, nil)
		streamClient2.EXPECT().Recv().Times(1).Return(nil, io.EOF)
		streamClient2.EXPECT().Send(req).Times(3).Return(nil)
		streamClient2.EXPECT().CloseSend().Times(1).Return(nil)
		client2.EXPECT().PublishBuildToolEventStream(ctx).Times(1).Return(streamClient2, nil)

		eventStream := buildv1_mock.NewMockPublishBuildEvent_PublishBuildToolEventStreamServer(ctrl)
		eventStream.EXPECT().Context().Times(1).Return(ctx)
		eventStream.EXPECT().Recv().Times(3).Return(req, nil)
		eventStream.EXPECT().Recv().Times(1).Return(nil, io.EOF)
		eventStream.EXPECT().Send(res).Times(3).Return(nil)

		err = bepFanner.PublishBuildToolEventStream(eventStream)
		Expect(err).To(BeNil())
	})
})
