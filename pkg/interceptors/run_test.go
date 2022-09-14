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

package interceptors

import (
	"context"
	"fmt"
	"log"
	"os"

	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"
)

func ExampleRun_interceptor_order() {
	g := NewGomega(func(message string, callerSkip ...int) {
		log.Fatal(message)
	})

	ctx := context.Background()
	args := []string{"foo", "bar"}
	os.Args = append([]string{"fake"}, args...)
	cmd := &cobra.Command{Use: "fake"}
	cmd.RunE = Run(
		[]Interceptor{
			LoggingInterceptor1(),
			InjectIntoContextInterceptor(),
			LoggingInterceptor2(),
		},
		func(_ctx context.Context, _cmd *cobra.Command, _args []string) error {
			g.Expect(_cmd).To(Equal(cmd))
			g.Expect(_args).To(Equal(args))
			ctxVal := _ctx.Value("my_key").(string)
			fmt.Printf("called %q with %v, and ctx contains %q\n", _cmd.Use, _args, ctxVal)
			return nil
		},
	)
	cmd.ExecuteContext(ctx)

	// Output:
	// interceptor 1
	// interceptor 2: "injected value"
	// called "fake" with [foo bar], and ctx contains "injected value"
	// interceptor 2: "injected value"
	// interceptor 1
}

func ExampleRun_nointerceptors() {
	g := NewGomega(func(message string, callerSkip ...int) {
		log.Fatal(message)
	})

	ctx := context.Background()
	args := []string{"foo", "bar"}
	os.Args = append([]string{"fake"}, args...)
	cmd := &cobra.Command{Use: "fake"}
	cmd.RunE = Run(
		[]Interceptor{},
		func(_ctx context.Context, _cmd *cobra.Command, _args []string) error {
			g.Expect(_cmd).To(Equal(cmd))
			g.Expect(_args).To(Equal(args))
			fmt.Printf("called %q with %v\n", _cmd.Use, _args)
			return nil
		},
	)
	cmd.ExecuteContext(ctx)

	// Output:
	// called "fake" with [foo bar]
}

func LoggingInterceptor1() Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next RunEContextFn) error {
		fmt.Println("interceptor 1")
		defer func() {
			fmt.Println("interceptor 1")
		}()
		return next(ctx, cmd, args)
	}
}

func InjectIntoContextInterceptor() Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next RunEContextFn) error {
		ctx = context.WithValue(ctx, "my_key", "injected value")
		return next(ctx, cmd, args)
	}
}

func LoggingInterceptor2() Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next RunEContextFn) error {
		fmt.Printf("interceptor 2: %q\n", ctx.Value("my_key").(string))
		defer func() {
			fmt.Printf("interceptor 2: %q\n", ctx.Value("my_key").(string))
		}()
		return next(ctx, cmd, args)
	}
}
