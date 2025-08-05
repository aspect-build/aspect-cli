package telemetry

import (
	"context"
	"os"

	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/sdk/resource"
	"go.opentelemetry.io/otel/sdk/trace"
	semconv "go.opentelemetry.io/otel/semconv/v1.34.0"

	"go.opentelemetry.io/otel/exporters/stdout/stdouttrace"
)

const (
	outputFileEnv = "ASPECT_OTEL_OUT"
)

/**
 * Configure global OpenTelemetry settings for the CLI.
 */
func StartSession(ctx context.Context) func() {
	if os.Getenv(outputFileEnv) != "" {
		des, err := setupOTelFile(ctx)
		if err != nil {
			panic(err)
		}
		return des
	}
	return func() {}
}

func setupOTelFile(ctx context.Context) (func(), error) {
	r, err := resource.Merge(
		resource.Default(),
		resource.NewWithAttributes(
			semconv.SchemaURL,
			semconv.ServiceName("Aspect CLI"),
		),
	)
	if err != nil {
		return nil, err
	}

	f, err := os.OpenFile(os.Getenv(outputFileEnv), os.O_CREATE|os.O_WRONLY, 0644)
	if err != nil {
		return nil, err
	}

	exp, err := stdouttrace.New(stdouttrace.WithWriter(f))
	if err != nil {
		return nil, err
	}

	tp := trace.NewTracerProvider(
		trace.WithBatcher(exp),
		trace.WithResource(r),
	)

	otel.SetTracerProvider(tp)

	return func() {
		tp.Shutdown(ctx)
	}, nil
}
