/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
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

func ExampleRun() {
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
